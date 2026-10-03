use crate::auth::sapisid::generate_sapisid_hash;
use crate::models::{Playlist, SourceTrack};
use crate::providers::traits::MusicProvider;
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, COOKIE, USER_AGENT};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct YouTubeMusicProvider {
    client: reqwest::Client,
    cookie_str: String,
    sapisid: String,
}

impl YouTubeMusicProvider {
    pub fn new(token: String) -> Self {
        let (cookie_str, sapisid) = if token.contains('=') {
            let extracted = token
                .split(';')
                .find_map(|part| {
                    let mut kv = part.trim().splitn(2, '=');
                    let k = kv.next()?.trim();
                    let v = kv.next()?.trim();
                    if k == "SAPISID" {
                        Some(v.to_string())
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    token.split(';').find_map(|part| {
                        let mut kv = part.trim().splitn(2, '=');
                        let k = kv.next()?.trim();
                        let v = kv.next()?.trim();
                        if k == "__Secure-3PAPISID" {
                            Some(v.to_string())
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or_else(|| token.clone());
            (token, extracted)
        } else {
            let cookie_str = format!("SAPISID={}; __Secure-3PAPISID={}", token, token);
            let sapisid = token;
            (cookie_str, sapisid)
        };

        Self {
            client: reqwest::Client::builder()
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            cookie_str,
            sapisid,
        }
    }

    fn build_headers(&self) -> Result<HeaderMap, String> {
        let mut headers = HeaderMap::new();
        let origin = "https://music.youtube.com";
        let auth_val = generate_sapisid_hash(&self.sapisid, origin);

        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&auth_val).map_err(|e| e.to_string())?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert("Origin", HeaderValue::from_static("https://music.youtube.com"));
        headers.insert("Referer", HeaderValue::from_static("https://music.youtube.com/"));
        headers.insert("X-Origin", HeaderValue::from_static("https://music.youtube.com"));
        headers.insert("X-Goog-AuthUser", HeaderValue::from_static("0"));

        if let Ok(c) = HeaderValue::from_str(&self.cookie_str) {
            headers.insert(COOKIE, c);
        }

        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36",
            ),
        );

        Ok(headers)
    }

    fn client_context() -> Value {
        json!({
            "client": {
                "clientName": "WEB_REMIX",
                "clientVersion": "1.20240101.01.00",
                "hl": "en",
                "gl": "US"
            }
        })
    }

    pub fn parse_duration_to_ms(duration_str: &str) -> u64 {
        let parts: Vec<&str> = duration_str.trim().split(':').collect();
        match parts.len() {
            1 => parts[0].parse::<u64>().unwrap_or(0) * 1000,
            2 => {
                let mins = parts[0].parse::<u64>().unwrap_or(0);
                let secs = parts[1].parse::<u64>().unwrap_or(0);
                (mins * 60 + secs) * 1000
            }
            3 => {
                let hours = parts[0].parse::<u64>().unwrap_or(0);
                let mins = parts[1].parse::<u64>().unwrap_or(0);
                let secs = parts[2].parse::<u64>().unwrap_or(0);
                (hours * 3600 + mins * 60 + secs) * 1000
            }
            _ => 0,
        }
    }

    fn parse_list_item_renderer(renderer: &Value) -> Option<SourceTrack> {
        let video_id = renderer["playlistItemData"]["videoId"]
            .as_str()
            .or_else(|| renderer["navigationEndpoint"]["watchEndpoint"]["videoId"].as_str())
            .or_else(|| {
                renderer["flexColumns"][0]["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"][0]["navigationEndpoint"]["watchEndpoint"]["videoId"].as_str()
            })?
            .to_string();

        let flex_cols = renderer["flexColumns"].as_array()?;
        let title = flex_cols
            .first()?
            ["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"][0]["text"]
            .as_str()?
            .to_string();

        let mut artists = Vec::new();
        let mut album = None;
        let mut duration_ms = 0;

        if flex_cols.len() > 1 {
            if let Some(runs) = flex_cols[1]["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"].as_array() {
                let mut artist_mode = true;
                for run in runs {
                    let text = run["text"].as_str().unwrap_or("").trim();
                    if text == "•" || text.is_empty() {
                        artist_mode = false;
                        continue;
                    }

                    if artist_mode {
                        if !text.is_empty() && text != "," {
                            artists.push(text.to_string());
                        }
                    } else if album.is_none() && !text.chars().all(|c| c.is_numeric() || c == ':') {
                        album = Some(text.to_string());
                    } else if text.contains(':') && duration_ms == 0 {
                        duration_ms = Self::parse_duration_to_ms(text);
                    }
                }
            }
        }

        if duration_ms == 0 {
            if let Some(fixed_cols) = renderer["fixedColumns"].as_array() {
                if let Some(runs) = fixed_cols.first().and_then(|c| c["musicResponsiveListItemFixedColumnRenderer"]["text"]["runs"].as_array()) {
                    if let Some(dur_text) = runs.first().and_then(|r| r["text"].as_str()) {
                        duration_ms = Self::parse_duration_to_ms(dur_text);
                    }
                }
            }
        }

        let is_explicit = renderer["badges"].as_array().is_some_and(|badges| {
            badges.iter().any(|b| {
                b["musicInlineBadgeRenderer"]["icon"]["iconType"]
                    .as_str()
                    .is_some_and(|icon| icon.contains("EXPLICIT"))
            })
        });

        let is_playable = !renderer["musicItemRendererDisplayPolicy"]
            .as_str()
            .is_some_and(|policy| policy.contains("GREY_OUT") || policy.contains("DISABLED"));

        let thumbnail_url = renderer["thumbnail"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"]
            .as_array()
            .and_then(|arr| arr.last())
            .and_then(|img| img["url"].as_str())
            .map(|s| s.to_string());

        Some(SourceTrack {
            id: video_id,
            title,
            artists,
            album,
            duration_ms,
            isrc: None,
            is_explicit,
            is_playable,
            preview_url: None,
            thumbnail_url,
        })
    }
}

#[async_trait]
impl MusicProvider for YouTubeMusicProvider {
    async fn list_playlists(&self) -> Result<Vec<Playlist>, String> {
        let headers = self.build_headers()?;
        let body = json!({
            "context": Self::client_context(),
            "browseId": "FEmusic_liked_playlists"
        });

        let res = self
            .client
            .post("https://music.youtube.com/youtubei/v1/browse")
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("InnerTube browse playlists request failed: {}", e))?;

        if !res.status().is_success() {
            return Err(format!("InnerTube browse failed with status: {}", res.status()));
        }

        let json: Value = res
            .json()
            .await
            .map_err(|e| format!("Failed to parse InnerTube browse response: {}", e))?;

        let mut playlists = Vec::new();

        // Search recursively for musicTwoRowItemRenderer objects in response
        if let Some(items) = json.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
            .and_then(|arr| arr.as_array())
        {
            for section in items {
                let renderers = section.pointer("/gridRenderer/items")
                    .or_else(|| section.pointer("/musicShelfRenderer/contents"))
                    .and_then(|arr| arr.as_array());

                if let Some(renderer_list) = renderers {
                    for item in renderer_list {
                        let row = &item["musicTwoRowItemRenderer"];
                        if row.is_null() {
                            continue;
                        }

                        let browse_id = match row["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str() {
                            Some(id) => id.to_string(),
                            None => continue,
                        };

                        let clean_id = browse_id.trim_start_matches("VL").to_string();
                        let title = row["title"]["runs"][0]["text"].as_str().unwrap_or("Untitled").to_string();

                        let mut description = None;
                        let mut track_count = 0;

                        if let Some(sub_runs) = row["subtitle"]["runs"].as_array() {
                            let text: String = sub_runs.iter().filter_map(|r| r["text"].as_str()).collect();
                            description = Some(text.clone());
                            // Attempt extracting count digits (handling comma-separated numbers like "1,054 tracks")
                            for word in text.split_whitespace() {
                                let clean = word.replace(',', "");
                                if let Ok(num) = clean.parse::<u32>() {
                                    track_count = num;
                                }
                            }
                        }

                        let cover_url = row["thumbnailRenderer"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"]
                            .as_array()
                            .and_then(|arr| arr.last())
                            .and_then(|img| img["url"].as_str())
                            .map(|s| s.to_string());

                        playlists.push(Playlist {
                            id: clean_id,
                            service: "ytmusic".to_string(),
                            title,
                            description,
                            track_count,
                            is_public: true,
                            cover_url,
                        });
                    }
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
        let headers = self.build_headers()?;
        let browse_id = if playlist_id.starts_with("VL") {
            playlist_id.to_string()
        } else {
            format!("VL{}", playlist_id)
        };

        let body = json!({
            "context": Self::client_context(),
            "browseId": browse_id
        });

        let res = self
            .client
            .post("https://music.youtube.com/youtubei/v1/browse")
            .headers(headers.clone())
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("InnerTube get_playlist_tracks failed: {}", e))?;

        if !res.status().is_success() {
            return Err(format!("InnerTube get tracks failed with status: {}", res.status()));
        }

        let json: Value = res
            .json()
            .await
            .map_err(|e| format!("Failed to parse playlist browse response: {}", e))?;

        let mut all_tracks = Vec::new();
        let mut continuation_token: Option<String> = None;

        // Parse initial batch - support both twoColumnBrowseResultsRenderer (desktop web)
        // and singleColumnBrowseResultsRenderer (mobile/tablet layout)
        let shelf = json.pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicPlaylistShelfRenderer")
            .or_else(|| json.pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicShelfRenderer"))
            .or_else(|| json.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicPlaylistShelfRenderer"))
            .or_else(|| json.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicShelfRenderer"));

        if let Some(shelf_obj) = shelf {
            if let Some(items) = shelf_obj["contents"].as_array() {
                let mut chunk = Vec::new();
                for item in items {
                    let renderer = &item["musicResponsiveListItemRenderer"];
                    if !renderer.is_null() {
                        if let Some(track) = Self::parse_list_item_renderer(renderer) {
                            chunk.push(track);
                        }
                    }
                }

                if let Some(ref sender) = tx {
                    let _ = sender.send(chunk.clone()).await;
                }

                all_tracks.extend(chunk);

                // Continuation token can be in continuations array or embedded continuationItemRenderer
                continuation_token = shelf_obj.pointer("/continuations/0/nextContinuationData/continuation")
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| {
                        items.iter().find_map(|item| {
                            item.pointer("/continuationItemRenderer/continuationEndpoint/continuationCommand/token")
                                .and_then(|t| t.as_str())
                                .map(|s| s.to_string())
                        })
                    });
            }
        }

        // Paginate continuations in 100-track chunks per D-02
        while let Some(token) = continuation_token {
            let url = format!(
                "https://music.youtube.com/youtubei/v1/browse?continuation={}&type=next",
                token
            );
            let cont_body = json!({
                "context": Self::client_context(),
            });

            let cont_res = self
                .client
                .post(&url)
                .headers(headers.clone())
                .json(&cont_body)
                .send()
                .await
                .map_err(|e| format!("InnerTube continuation failed: {}", e))?;

            if !cont_res.status().is_success() {
                break;
            }

            let cont_json: Value = cont_res.json().await.map_err(|e| e.to_string())?;

            let cont_items = cont_json.pointer("/continuationContents/musicPlaylistShelfContinuation/contents")
                .or_else(|| cont_json.pointer("/continuationContents/musicShelfContinuation/contents"))
                .or_else(|| cont_json.pointer("/onResponseReceivedActions/0/appendContinuationItemsAction/continuationItems"))
                .and_then(|c| c.as_array());

            if let Some(items) = cont_items {
                let mut chunk = Vec::new();
                for item in items {
                    let renderer = &item["musicResponsiveListItemRenderer"];
                    if !renderer.is_null() {
                        if let Some(track) = Self::parse_list_item_renderer(renderer) {
                            chunk.push(track);
                        }
                    }
                }

                if !chunk.is_empty() {
                    if let Some(ref sender) = tx {
                        let _ = sender.send(chunk.clone()).await;
                    }
                    all_tracks.extend(chunk);
                }

                continuation_token = cont_json.pointer("/continuationContents/musicPlaylistShelfContinuation/continuations/0/nextContinuationData/continuation")
                    .or_else(|| cont_json.pointer("/continuationContents/musicShelfContinuation/continuations/0/nextContinuationData/continuation"))
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| {
                        items.iter().find_map(|item| {
                            item.pointer("/continuationItemRenderer/continuationEndpoint/continuationCommand/token")
                                .and_then(|t| t.as_str())
                                .map(|s| s.to_string())
                        })
                    });
            } else {
                break;
            }
        }

        Ok(all_tracks)
    }

    async fn search_track(&self, query: &str) -> Result<Vec<SourceTrack>, String> {
        let headers = self.build_headers()?;

        // Primary search: Songs filter per D-05
        let body = json!({
            "context": Self::client_context(),
            "query": query,
            "params": "Eg-KAQwIARAAGAAgACgAMABqChAMEAMQBBAFEAo%3D"
        });

        let res = self
            .client
            .post("https://music.youtube.com/youtubei/v1/search")
            .headers(headers.clone())
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("InnerTube search failed: {}", e))?;

        let mut results = Vec::new();

        if res.status().is_success() {
            if let Ok(json) = res.json::<Value>().await {
                // Parse top result and songs shelf items per D-07
                if let Some(sections) = json.pointer("/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
                    .and_then(|arr| arr.as_array())
                {
                    for section in sections {
                        if let Some(contents) = section.pointer("/musicCardShelfRenderer/contents")
                            .or_else(|| section.pointer("/musicShelfRenderer/contents"))
                            .and_then(|arr| arr.as_array())
                        {
                            for item in contents.iter().take(5) {
                                let renderer = &item["musicResponsiveListItemRenderer"];
                                if !renderer.is_null() {
                                    if let Some(track) = Self::parse_list_item_renderer(renderer) {
                                        results.push(track);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Secondary fallback: Video search if songs filter yielded no candidate per D-05
        if results.is_empty() {
            let video_body = json!({
                "context": Self::client_context(),
                "query": query,
                "params": "Eg-KAQwIAhABGAAgACgAMABqChAMEAMQBBAFEAo%3D"
            });

            if let Ok(video_res) = self
                .client
                .post("https://music.youtube.com/youtubei/v1/search")
                .headers(headers)
                .json(&video_body)
                .send()
                .await
            {
                if video_res.status().is_success() {
                    if let Ok(video_json) = video_res.json::<Value>().await {
                        if let Some(sections) = video_json.pointer("/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
                            .and_then(|arr| arr.as_array())
                        {
                            for section in sections {
                                if let Some(contents) = section.pointer("/musicShelfRenderer/contents").and_then(|arr| arr.as_array()) {
                                    for item in contents.iter().take(5) {
                                        let renderer = &item["musicResponsiveListItemRenderer"];
                                        if !renderer.is_null() {
                                            if let Some(track) = Self::parse_list_item_renderer(renderer) {
                                                results.push(track);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    async fn create_playlist(&self, title: &str, description: Option<&str>) -> Result<String, String> {
        let headers = self.build_headers()?;
        let body = json!({
            "context": Self::client_context(),
            "title": title,
            "description": description.unwrap_or(""),
            "privacyStatus": "PRIVATE"
        });

        let res = self
            .client
            .post("https://music.youtube.com/youtubei/v1/playlist/create")
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("InnerTube playlist creation failed: {}", e))?;

        if !res.status().is_success() {
            return Err(format!("Playlist creation failed with status: {}", res.status()));
        }

        let json: Value = res.json().await.map_err(|e| e.to_string())?;
        let playlist_id = json["playlistId"]
            .as_str()
            .ok_or_else(|| "Failed to extract created playlist ID".to_string())?;

        Ok(playlist_id.to_string())
    }

    async fn add_tracks_to_playlist(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<(), String> {
        let headers = self.build_headers()?;
        let clean_id = playlist_id.trim_start_matches("VL");

        let actions: Vec<Value> = track_ids
            .iter()
            .map(|id| {
                json!({
                    "action": "ACTION_ADD_VIDEO",
                    "addedVideoId": id
                })
            })
            .collect();

        let body = json!({
            "context": Self::client_context(),
            "playlistId": clean_id,
            "actions": actions
        });

        let res = self
            .client
            .post("https://music.youtube.com/youtubei/v1/browse/edit_playlist")
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("InnerTube edit_playlist failed: {}", e))?;

        if !res.status().is_success() {
            return Err(format!("Add tracks failed with status: {}", res.status()));
        }

        Ok(())
    }

    async fn remove_tracks_from_playlist(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<(), String> {
        let headers = self.build_headers()?;
        let clean_id = playlist_id.trim_start_matches("VL");

        let actions: Vec<Value> = track_ids
            .iter()
            .map(|id| {
                json!({
                    "action": "ACTION_REMOVE_VIDEO_BY_VIDEO_ID",
                    "removedVideoId": id
                })
            })
            .collect();

        let body = json!({
            "context": Self::client_context(),
            "playlistId": clean_id,
            "actions": actions
        });

        let res = self
            .client
            .post("https://music.youtube.com/youtubei/v1/browse/edit_playlist")
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("InnerTube edit_playlist remove failed: {}", e))?;

        if !res.status().is_success() {
            return Err(format!("Remove tracks failed with status: {}", res.status()));
        }

        Ok(())
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<(), String> {
        let headers = self.build_headers()?;
        let clean_id = playlist_id.trim_start_matches("VL");

        let body = json!({
            "context": Self::client_context(),
            "playlistId": clean_id
        });

        let res = self
            .client
            .post("https://music.youtube.com/youtubei/v1/playlist/delete")
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("InnerTube playlist/delete failed: {}", e))?;

        if !res.status().is_success() {
            return Err(format!("Delete playlist failed with status: {}", res.status()));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_to_ms() {
        assert_eq!(YouTubeMusicProvider::parse_duration_to_ms("3:45"), 225_000);
        assert_eq!(YouTubeMusicProvider::parse_duration_to_ms("0:30"), 30_000);
        assert_eq!(YouTubeMusicProvider::parse_duration_to_ms("1:02:15"), 3_735_000);
        assert_eq!(YouTubeMusicProvider::parse_duration_to_ms("45"), 45_000);
    }

    #[tokio::test]
    #[ignore]
    async fn test_live_ytmusic_diag() {
        let app_data = std::env::var("APPDATA").unwrap_or_default();
        let db_path = format!("{}\\com.soundshift.app\\soundshift.db", app_data);
        if let Ok(conn) = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            if let Ok(Some(token)) = crate::auth::keyring_store::retrieve_credential("ytmusic", &conn) {
                println!("FOUND YTMUSIC TOKEN (len={})", token.len());
                let provider = YouTubeMusicProvider::new(token);

                let playlists = provider.list_playlists().await.expect("Failed to list playlists");
                println!("FOUND {} PLAYLISTS", playlists.len());
                assert!(!playlists.is_empty(), "Expected at least 1 playlist");
            } else {
                println!("NO YTMUSIC CREDENTIAL IN DB!");
            }
        }
    }
}
