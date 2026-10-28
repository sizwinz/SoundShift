use crate::auth::sapisid::generate_sapisid_hash;
use crate::models::{Playlist, SourceTrack};
use crate::providers::contracts::{
    MutationOutcome, OccurrenceOutcome, OccurrenceStatus, PlaylistOccurrence, PlaylistRead,
};
use crate::providers::error::{ProviderError, ProviderErrorKind, SendCertainty};
use crate::providers::traits::MusicProvider;
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, COOKIE, USER_AGENT};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct YouTubeMusicProvider {
    client: reqwest::Client,
    cookie_str: String,
    sapisid: String,
    #[cfg(test)]
    edit_url: Option<String>,
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
                .connect_timeout(std::time::Duration::from_secs(12))
                .timeout(std::time::Duration::from_secs(30))
                .tcp_keepalive(std::time::Duration::from_secs(30))
                .pool_idle_timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            cookie_str,
            sapisid,
            #[cfg(test)]
            edit_url: None,
        }
    }

    async fn fetch_playlist(
        &self,
        playlist_id: &str,
        tx: Option<tokio::sync::mpsc::Sender<Vec<SourceTrack>>>,
    ) -> Result<PlaylistRead, ProviderError> {
        let headers = self.build_headers().map_err(ProviderError::before_send)?;
        let browse_id = if playlist_id.starts_with("VL") {
            playlist_id.to_string()
        } else {
            format!("VL{}", playlist_id)
        };

        let body = json!({
            "context": Self::client_context(),
            "browseId": browse_id
        });

        let endpoint = "https://music.youtube.com/youtubei/v1/browse";
        #[cfg(test)]
        let endpoint = self.edit_url.as_deref().unwrap_or(endpoint);
        let res = self
            .client
            .post(endpoint)
            .headers(headers.clone())
            .json(&body)
            .send()
            .await
            .map_err(ProviderError::read_transport)?;

        if !res.status().is_success() {
            return Err(ProviderError::read_http(
                res.status().as_u16(),
                res.headers(),
            ));
        }

        let json: Value = res.json().await.map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::NotSent,
                "Invalid playlist response schema",
            )
        })?;

        let mut all_tracks = Vec::new();
        let mut remote_ids = Vec::new();
        let mut continuation_token: Option<String>;

        // Parse initial batch - support both twoColumnBrowseResultsRenderer (desktop web)
        // and singleColumnBrowseResultsRenderer (mobile/tablet layout)
        let shelf = json.pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicPlaylistShelfRenderer")
            .or_else(|| json.pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicShelfRenderer"))
            .or_else(|| json.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicPlaylistShelfRenderer"))
            .or_else(|| json.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicShelfRenderer"));

        let shelf_obj = shelf.ok_or_else(|| {
            ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::NotSent,
                "Playlist schema mismatch; incomplete read",
            )
        })?;
        {
            if let Some(items) = shelf_obj["contents"].as_array() {
                let mut chunk = Vec::new();
                for item in items {
                    let renderer = &item["musicResponsiveListItemRenderer"];
                    if item.get("continuationItemRenderer").is_none() {
                        chunk.push(Self::parse_list_item_renderer(renderer).unwrap_or_else(|| {
                            SourceTrack::unavailable(all_tracks.len() + chunk.len())
                        }));
                        remote_ids.push(
                            renderer
                                .pointer("/playlistItemData/playlistSetVideoId")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned),
                        );
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
            } else {
                return Err(ProviderError::new(
                    ProviderErrorKind::MalformedResponse,
                    SendCertainty::NotSent,
                    "Playlist item schema mismatch; incomplete read",
                ));
            }
        }

        // Paginate continuations in 100-track chunks per D-02
        let mut seen_continuations = std::collections::HashSet::new();
        while let Some(token) = continuation_token {
            if !seen_continuations.insert(token.clone()) {
                return Err(ProviderError::new(
                    ProviderErrorKind::Pagination,
                    SendCertainty::NotSent,
                    "Repeated playlist continuation; incomplete read",
                ));
            }
            let url = format!(
                "https://music.youtube.com/youtubei/v1/browse?continuation={}&type=next",
                token
            );
            let cont_body = json!({
                "context": Self::client_context(),
            });

            #[cfg(test)]
            let url = self.edit_url.clone().unwrap_or(url);
            let cont_res = self
                .client
                .post(&url)
                .headers(headers.clone())
                .json(&cont_body)
                .send()
                .await
                .map_err(ProviderError::read_transport)?;

            if !cont_res.status().is_success() {
                return Err(ProviderError::read_http(
                    cont_res.status().as_u16(),
                    cont_res.headers(),
                ));
            }
            let cont_json: Value = cont_res.json().await.map_err(|_| {
                ProviderError::new(
                    ProviderErrorKind::MalformedResponse,
                    SendCertainty::NotSent,
                    "Invalid continuation response schema",
                )
            })?;

            let cont_items = cont_json.pointer("/continuationContents/musicPlaylistShelfContinuation/contents")
                .or_else(|| cont_json.pointer("/continuationContents/musicShelfContinuation/contents"))
                .or_else(|| cont_json.pointer("/onResponseReceivedActions/0/appendContinuationItemsAction/continuationItems"))
                .and_then(|c| c.as_array());

            if let Some(items) = cont_items {
                let mut chunk = Vec::new();
                for item in items {
                    let renderer = &item["musicResponsiveListItemRenderer"];
                    if item.get("continuationItemRenderer").is_none() {
                        chunk.push(Self::parse_list_item_renderer(renderer).unwrap_or_else(|| {
                            SourceTrack::unavailable(all_tracks.len() + chunk.len())
                        }));
                        remote_ids.push(
                            renderer
                                .pointer("/playlistItemData/playlistSetVideoId")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned),
                        );
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
                return Err(ProviderError::new(
                    ProviderErrorKind::MalformedResponse,
                    SendCertainty::NotSent,
                    "Playlist continuation schema mismatch; incomplete read",
                ));
            }
        }

        Ok(PlaylistRead {
            occurrences: all_tracks
                .into_iter()
                .zip(remote_ids)
                .enumerate()
                .map(|(index, (track, remote_id))| PlaylistOccurrence {
                    index,
                    track,
                    remote_id,
                })
                .collect(),
            revision: None,
            complete: true,
        })
    }

    fn removal_actions(occurrences: &[PlaylistOccurrence]) -> Result<Value, ProviderError> {
        let mut seen = std::collections::HashSet::new();
        let mut actions = Vec::new();
        for occurrence in occurrences {
            let remote = occurrence
                .remote_id
                .as_deref()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| {
                    ProviderError::new(
                        ProviderErrorKind::Unknown,
                        SendCertainty::NotSent,
                        "Occurrence identity missing; no removal sent",
                    )
                })?;
            if !seen.insert(remote) {
                return Err(ProviderError::new(
                    ProviderErrorKind::Conflict,
                    SendCertainty::NotSent,
                    "Duplicate occurrence identity; no removal sent",
                ));
            }
            actions.push(json!({"action":"ACTION_REMOVE_VIDEO","removedVideoId":occurrence.track.id,"setVideoId":remote}));
        }
        Ok(Value::Array(actions))
    }

    fn parse_add_outcome(json: &Value, ids: &[String]) -> MutationOutcome {
        let results = json["playlistEditResults"].as_array();
        let mut seen = std::collections::HashSet::new();
        let occurrences = ids
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let added = results
                    .and_then(|r| r.get(index))
                    .map(|r| &r["playlistEditVideoAddedResultData"]);
                let remote_id = added
                    .filter(|a| a["videoId"].as_str() == Some(id.as_str()))
                    .and_then(|a| a["setVideoId"].as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let valid = json["status"].as_str() == Some("STATUS_SUCCEEDED")
                    && remote_id.as_ref().is_some_and(|r| seen.insert(r.clone()));
                OccurrenceOutcome {
                    index,
                    track_id: id.clone(),
                    status: if valid {
                        OccurrenceStatus::Accepted
                    } else {
                        OccurrenceStatus::Uncertain
                    },
                    remote_id: if valid { remote_id } else { None },
                }
            })
            .collect();
        MutationOutcome {
            occurrences,
            revision: None,
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
        headers.insert(
            "Origin",
            HeaderValue::from_static("https://music.youtube.com"),
        );
        headers.insert(
            "Referer",
            HeaderValue::from_static("https://music.youtube.com/"),
        );
        headers.insert(
            "X-Origin",
            HeaderValue::from_static("https://music.youtube.com"),
        );
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
                renderer["flexColumns"][0]["musicResponsiveListItemFlexColumnRenderer"]["text"]
                    ["runs"][0]["navigationEndpoint"]["watchEndpoint"]["videoId"]
                    .as_str()
            })?
            .to_string();

        let flex_cols = renderer["flexColumns"].as_array()?;
        let title = flex_cols.first()?["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"]
            [0]["text"]
            .as_str()?
            .to_string();

        let mut artists = Vec::new();
        let mut album = None;
        let mut duration_ms = 0;

        if flex_cols.len() > 1 {
            if let Some(runs) =
                flex_cols[1]["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"].as_array()
            {
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
                if let Some(runs) = fixed_cols.first().and_then(|c| {
                    c["musicResponsiveListItemFixedColumnRenderer"]["text"]["runs"].as_array()
                }) {
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

        let thumbnail_url = renderer["thumbnail"]["musicThumbnailRenderer"]["thumbnail"]
            ["thumbnails"]
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
    async fn check_playlist_editable(&self, playlist_id: &str) -> Result<(), ProviderError> {
        fn editable(value: &Value) -> bool {
            match value {
                Value::Object(map) => {
                    map.contains_key("musicEditablePlaylistDetailHeaderRenderer")
                        || map.values().any(editable)
                }
                Value::Array(items) => items.iter().any(editable),
                _ => false,
            }
        }
        if playlist_id.is_empty()
            || !playlist_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(ProviderError::new(
                ProviderErrorKind::Permission,
                SendCertainty::NotSent,
                "Choose an editable YouTube Music playlist",
            ));
        }
        let endpoint = "https://music.youtube.com/youtubei/v1/browse";
        #[cfg(test)]
        let endpoint = self.edit_url.as_deref().unwrap_or(endpoint);
        let response=self.client.post(endpoint).headers(self.build_headers().map_err(ProviderError::before_send)?)
            .json(&json!({"context":Self::client_context(),"browseId":if playlist_id.starts_with("VL"){playlist_id.to_string()}else{format!("VL{playlist_id}")}})).send().await.map_err(ProviderError::read_transport)?;
        if !response.status().is_success() {
            return Err(ProviderError::read_http(
                response.status().as_u16(),
                response.headers(),
            ));
        }
        let value: Value = response.json().await.map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::NotSent,
                "YouTube playlist permissions response is malformed",
            )
        })?;
        if !editable(&value) {
            return Err(ProviderError::new(
                ProviderErrorKind::Permission,
                SendCertainty::NotSent,
                "YouTube playlist is read-only or editability is unverified",
            ));
        }
        Ok(())
    }
    fn sync_capabilities(&self) -> crate::providers::contracts::SyncCapabilities {
        crate::providers::contracts::SyncCapabilities {
            duplicate_insertion: true,
            exact_removal: true,
            occurrence_movement: true,
        }
    }
    async fn move_occurrence(
        &self,
        playlist_id: &str,
        occurrence: &PlaylistOccurrence,
        before: Option<&PlaylistOccurrence>,
        baseline: &PlaylistRead,
    ) -> Result<MutationOutcome, ProviderError> {
        let identity = |o: &PlaylistOccurrence| -> Result<String, ProviderError> {
            if !baseline.complete
                || baseline
                    .occurrences
                    .get(o.index)
                    .is_none_or(|b| b.remote_id != o.remote_id || b.track.id != o.track.id)
            {
                return Err(ProviderError::new(
                    ProviderErrorKind::Conflict,
                    SendCertainty::NotSent,
                    "Occurrence differs from complete baseline",
                ));
            }
            o.remote_id
                .clone()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| {
                    ProviderError::new(
                        ProviderErrorKind::Unknown,
                        SendCertainty::NotSent,
                        "Occurrence identity missing; no move sent",
                    )
                })
        };
        let remote = identity(occurrence)?;
        let mut action = json!({"action":"ACTION_MOVE_VIDEO_BEFORE","setVideoId":remote});
        if let Some(successor) = before {
            action["movedSetVideoIdSuccessor"] = json!(identity(successor)?);
        }
        let endpoint = "https://music.youtube.com/youtubei/v1/browse/edit_playlist";
        #[cfg(test)]
        let endpoint = self.edit_url.as_deref().unwrap_or(endpoint);
        let response = self.client.post(endpoint).headers(self.build_headers().map_err(ProviderError::before_send)?)
            .json(&json!({"context":Self::client_context(),"playlistId":playlist_id.trim_start_matches("VL"),"actions":[action]}))
            .send().await.map_err(ProviderError::transport)?;
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
                "Invalid movement response; outcome uncertain",
            )
        })?;
        Ok(MutationOutcome {
            occurrences: vec![OccurrenceOutcome {
                index: 0,
                track_id: occurrence.track.id.clone(),
                remote_id: Some(remote),
                status: if value["status"] == "STATUS_SUCCEEDED" {
                    OccurrenceStatus::Accepted
                } else {
                    OccurrenceStatus::Uncertain
                },
            }],
            revision: None,
        })
    }
    async fn canary(&self) -> Result<(), String> {
        self.check_session().await.map_err(|e| e.to_string())
    }
    async fn check_session(&self) -> Result<(), ProviderError> {
        let headers = self.build_headers().map_err(ProviderError::before_send)?;
        let endpoint = "https://music.youtube.com/youtubei/v1/browse";
        #[cfg(test)]
        let endpoint = self.edit_url.as_deref().unwrap_or(endpoint);
        let response = self
            .client
            .post(endpoint)
            .headers(headers)
            .json(&json!({"context":Self::client_context(),"browseId":"FEmusic_liked_playlists"}))
            .send()
            .await
            .map_err(ProviderError::read_transport)?;
        if !response.status().is_success() {
            return Err(ProviderError::read_http(
                response.status().as_u16(),
                response.headers(),
            ));
        }
        let value: Value = response.json().await.map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::NotSent,
                "Session canary returned invalid JSON",
            )
        })?;
        if value.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents").and_then(|v|v.as_array()).is_none() {
            return Err(ProviderError::new(ProviderErrorKind::MalformedResponse,SendCertainty::NotSent,"Session canary schema mismatch; reconnect or review provider availability"));
        }
        Ok(())
    }

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
            return Err(format!(
                "InnerTube browse failed with status: {}",
                res.status()
            ));
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
        Ok(self
            .fetch_playlist(playlist_id, tx)
            .await
            .map_err(|e| e.to_string())?
            .occurrences
            .into_iter()
            .map(|o| o.track)
            .collect())
    }
    async fn read_playlist(&self, playlist_id: &str) -> Result<PlaylistRead, ProviderError> {
        let before = self.fetch_playlist(playlist_id, None).await?;
        let after = self.fetch_playlist(playlist_id, None).await?;
        if before.ids() != after.ids()
            || before
                .occurrences
                .iter()
                .map(|o| &o.remote_id)
                .ne(after.occurrences.iter().map(|o| &o.remote_id))
        {
            return Err(ProviderError::new(
                ProviderErrorKind::Conflict,
                SendCertainty::NotSent,
                "Playlist occurrence identity changed during read",
            ));
        }
        if after
            .occurrences
            .iter()
            .any(|o| o.track.id.starts_with("unavailable_") || o.track.id.is_empty())
        {
            return Err(ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::NotSent,
                "Unresolved playlist occurrences; complete baseline unavailable",
            ));
        }
        Ok(after)
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

    async fn create_playlist(
        &self,
        title: &str,
        description: Option<&str>,
    ) -> Result<String, String> {
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
            return Err(format!(
                "Playlist creation failed with status: {}",
                res.status()
            ));
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
        let outcome = self
            .add_occurrences(playlist_id, track_ids)
            .await
            .map_err(|e| e.to_string())?;
        if !outcome.all_accepted() {
            return Err("YouTube Music mutation outcome uncertain: missing occurrence evidence; reconcile before retry".into());
        }
        Ok(())
    }

    async fn add_occurrences(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<MutationOutcome, ProviderError> {
        let headers = self.build_headers().map_err(ProviderError::before_send)?;
        let clean_id = playlist_id.trim_start_matches("VL");

        let actions: Vec<Value> = track_ids
            .iter()
            .map(|id| {
                json!({
                    "action": "ACTION_ADD_VIDEO",
                    "addedVideoId": id,
                    "dedupeOption": "DEDUPE_OPTION_SKIP"
                })
            })
            .collect();

        let body = json!({
            "context": Self::client_context(),
            "playlistId": clean_id,
            "actions": actions
        });

        let endpoint = "https://music.youtube.com/youtubei/v1/browse/edit_playlist";
        #[cfg(test)]
        let endpoint = self.edit_url.as_deref().unwrap_or(endpoint);
        let res = self
            .client
            .post(endpoint)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(ProviderError::transport)?;

        if !res.status().is_success() {
            return Err(ProviderError::http(res.status().as_u16(), res.headers()));
        }

        let json: Value = res.json().await.map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::MayHaveApplied,
                "Invalid mutation response; remote outcome uncertain",
            )
        })?;

        Ok(Self::parse_add_outcome(&json, track_ids))
    }

    fn supports_occurrence_removal(&self) -> bool {
        true
    }
    async fn remove_occurrences(
        &self,
        playlist_id: &str,
        occurrences: &[PlaylistOccurrence],
        _revision: Option<&str>,
    ) -> Result<(), ProviderError> {
        let actions = Self::removal_actions(occurrences)?;
        let headers = self.build_headers().map_err(ProviderError::before_send)?;
        let endpoint = "https://music.youtube.com/youtubei/v1/browse/edit_playlist";
        #[cfg(test)]
        let endpoint = self.edit_url.as_deref().unwrap_or(endpoint);
        let response=self.client.post(endpoint).headers(headers).json(&json!({"context":Self::client_context(),"playlistId":playlist_id.trim_start_matches("VL"),"actions":actions})).send().await.map_err(ProviderError::transport)?;
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
                "Invalid removal response; verify remote post-state",
            )
        })?;
        if value["status"].as_str() != Some("STATUS_SUCCEEDED") {
            return Err(ProviderError::new(
                ProviderErrorKind::Unknown,
                SendCertainty::MayHaveApplied,
                "Removal response uncertain; verify before retry",
            ));
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
            return Err(format!(
                "Remove tracks failed with status: {}",
                res.status()
            ));
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
            return Err(format!(
                "Delete playlist failed with status: {}",
                res.status()
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn sync_youtube_editability_requires_recognized_editable_header() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut bytes = [0; 8192];
            socket.read(&mut bytes).unwrap();
            let body = r#"{"header":{"musicEditablePlaylistDetailHeaderRenderer":{}}}"#;
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        provider.check_playlist_editable("fixture").await.unwrap();
        thread.join().unwrap();
    }
    #[test]
    fn unknown_duplicate_insertion_status_is_not_confirmation() {
        let outcome = YouTubeMusicProvider::parse_add_outcome(
            &json!({"status":"NEW_UNKNOWN_STATUS","playlistEditResults":[
                {"playlistEditVideoAddedResultData":{"videoId":"a","setVideoId":"one"}},
                {"playlistEditVideoAddedResultData":{"videoId":"a","setVideoId":"two"}}
            ]}),
            &["a".into(), "a".into()],
        );
        assert!(outcome
            .occurrences
            .iter()
            .all(|o| o.status == OccurrenceStatus::Uncertain));
    }
    #[tokio::test]
    async fn sync_move_uses_occurrence_identity_and_successor() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut bytes = [0; 8192];
            let size = socket.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..size]).to_string();
            let body = r#"{"status":"STATUS_SUCCEEDED"}"#;
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            request
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        let occurrence = PlaylistOccurrence {
            index: 0,
            track: SourceTrack {
                id: "a".into(),
                ..SourceTrack::unavailable(0)
            },
            remote_id: Some("occ-a".into()),
        };
        let successor = PlaylistOccurrence {
            index: 1,
            track: SourceTrack {
                id: "b".into(),
                ..SourceTrack::unavailable(1)
            },
            remote_id: Some("occ-b".into()),
        };
        let baseline = PlaylistRead {
            occurrences: vec![occurrence.clone(), successor.clone()],
            revision: None,
            complete: true,
        };
        let outcome = provider
            .move_occurrence("fixture", &occurrence, Some(&successor), &baseline)
            .await
            .unwrap();
        assert!(outcome.all_accepted());
        let request = thread.join().unwrap();
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(
            body["actions"][0],
            json!({"action":"ACTION_MOVE_VIDEO_BEFORE","setVideoId":"occ-a","movedSetVideoIdSuccessor":"occ-b"})
        );
    }
    #[test]
    fn sync_capabilities_preserve_duplicates_without_claiming_spotify_exact_removal() {
        let yt = YouTubeMusicProvider::new("fixture".into());
        let spotify = crate::providers::spotify::SpotifyProvider::new("fixture".into());
        assert!(yt.sync_capabilities().duplicate_insertion);
        assert!(yt.sync_capabilities().occurrence_movement);
        assert!(yt.sync_capabilities().exact_removal);
        assert!(spotify.sync_capabilities().duplicate_insertion);
        assert!(spotify.sync_capabilities().occurrence_movement);
        assert!(!spotify.sync_capabilities().exact_removal);
    }

    #[tokio::test]
    async fn mutation_error_excludes_raw_response_body() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut request = [0u8; 8192];
            socket.read(&mut request).unwrap();
            let body = "Cookie: SAPISID=offline_secret_cookie\nBearer offline_secret_bearer";
            write!(
                socket,
                "HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        let error = provider
            .add_tracks_to_playlist("fixture", &["track".into()])
            .await
            .unwrap_err();
        thread.join().unwrap();
        assert!(error.contains("403"));
        assert!(
            !error.contains("offline_secret"),
            "provider error leaked its raw response: {error}"
        );
    }

    #[tokio::test]
    async fn success_without_occurrence_evidence_is_uncertain() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut request = [0u8; 8192];
            socket.read(&mut request).unwrap();
            let body = r#"{"status":"STATUS_SUCCEEDED"}"#;
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        let result = provider
            .add_tracks_to_playlist("fixture", &["track".into()])
            .await;
        thread.join().unwrap();
        assert!(
            result.is_err(),
            "a top-level success flag cannot prove accepted occurrences"
        );
    }

    #[test]
    fn partial_mutation_response_preserves_occurrence_certainty() {
        use crate::providers::contracts::OccurrenceStatus;
        let outcome = YouTubeMusicProvider::parse_add_outcome(
            &json!({"status":"STATUS_SUCCEEDED","playlistEditResults":[
                {"playlistEditVideoAddedResultData":{"videoId":"a","setVideoId":"occurrence-a"}}
            ]}),
            &["a".into(), "b".into()],
        );
        assert_eq!(outcome.occurrences.len(), 2);
        assert_eq!(outcome.occurrences[0].status, OccurrenceStatus::Accepted);
        assert_eq!(
            outcome.occurrences[0].remote_id.as_deref(),
            Some("occurrence-a")
        );
        assert_eq!(outcome.occurrences[1].status, OccurrenceStatus::Uncertain);
    }

    #[tokio::test]
    async fn malformed_playlist_read_is_not_an_empty_success() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut request = [0u8; 8192];
            socket.read(&mut request).unwrap();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        let result = provider.get_playlist_tracks("fixture", None).await;
        thread.join().unwrap();
        assert!(
            result.is_err(),
            "missing playlist schema must block mutation, not become an empty baseline"
        );
    }

    #[tokio::test]
    async fn repeated_continuation_is_rejected_before_another_request() {
        use std::io::{Read, Write};
        use std::sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        };
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        server.set_nonblocking(true).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let stop = done.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let thread = std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let Ok((mut socket, _)) = server.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    continue;
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0u8; 8192];
                socket.read(&mut request).unwrap();
                let n = counted.fetch_add(1, Ordering::SeqCst);
                let body = if n == 0 { json!({"contents":{"twoColumnBrowseResultsRenderer":{"secondaryContents":{"sectionListRenderer":{"contents":[{"musicPlaylistShelfRenderer":{"contents":[],"continuations":[{"nextContinuationData":{"continuation":"repeated"}}]}}]}}}}}) }
                    else if n == 1 { json!({"continuationContents":{"musicPlaylistShelfContinuation":{"contents":[],"continuations":[{"nextContinuationData":{"continuation":"repeated"}}]}}}) }
                    else { json!({}) }.to_string();
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            }
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        let result = provider.get_playlist_tracks("fixture", None).await;
        done.store(true, Ordering::SeqCst);
        thread.join().unwrap();
        assert!(result.is_err());
        assert!(calls.load(Ordering::SeqCst) <= 2);
    }

    #[tokio::test]
    async fn failed_continuation_does_not_return_partial_playlist() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut socket, _) = server.accept().unwrap();
                let mut request = [0u8; 8192];
                socket.read(&mut request).unwrap();
                let body = if index == 0 {
                    json!({"contents":{"twoColumnBrowseResultsRenderer":{"secondaryContents":{"sectionListRenderer":{"contents":[{"musicPlaylistShelfRenderer":{"contents":[],"continuations":[{"nextContinuationData":{"continuation":"next"}}]}}]}}}}}).to_string()
                } else {
                    "{}".into()
                };
                let status = if index == 0 {
                    "200 OK"
                } else {
                    "503 Unavailable"
                };
                write!(socket,"HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",status,body.len(),body).unwrap();
            }
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        let result = provider.get_playlist_tracks("fixture", None).await;
        thread.join().unwrap();
        assert!(
            result.is_err(),
            "partial playlist cannot be used as a complete baseline"
        );
    }

    #[tokio::test]
    async fn unparseable_playlist_occurrence_keeps_its_position() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut request = [0u8; 8192];
            socket.read(&mut request).unwrap();
            let body = json!({"contents":{"twoColumnBrowseResultsRenderer":{"secondaryContents":{"sectionListRenderer":{"contents":[{"musicPlaylistShelfRenderer":{"contents":[{"musicResponsiveListItemRenderer":{}}]}}]}}}}}).to_string();
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        let tracks = provider.get_playlist_tracks("fixture", None).await.unwrap();
        thread.join().unwrap();
        assert_eq!(tracks.len(), 1);
        assert!(!tracks[0].is_playable);
        assert_eq!(tracks[0].id, "unavailable_0");
    }

    #[tokio::test]
    async fn mutation_returns_typed_rate_limit_with_retry_guidance() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut request = [0u8; 8192];
            socket.read(&mut request).unwrap();
            write!(socket,"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 60\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        let error = provider
            .add_occurrences("fixture", &["a".into()])
            .await
            .unwrap_err();
        thread.join().unwrap();
        assert_eq!(
            error.kind,
            crate::providers::error::ProviderErrorKind::RateLimited
        );
        assert_eq!(error.retry_after_ms, Some(60_000));
        assert_eq!(
            error.certainty,
            crate::providers::error::SendCertainty::Rejected
        );
    }

    #[tokio::test]
    async fn playlist_read_preserves_youtube_occurrence_identity() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut socket, _) = server.accept().unwrap();
                let mut request = [0u8; 8192];
                socket.read(&mut request).unwrap();
                let body=json!({"contents":{"twoColumnBrowseResultsRenderer":{"secondaryContents":{"sectionListRenderer":{"contents":[{"musicPlaylistShelfRenderer":{"contents":[{"musicResponsiveListItemRenderer":{"playlistItemData":{"videoId":"a","playlistSetVideoId":"owned-a"},"flexColumns":[{"musicResponsiveListItemFlexColumnRenderer":{"text":{"runs":[{"text":"fixture"}]}}}]}}]}}]}}}}}).to_string();
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            }
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        let read = provider.read_playlist("fixture").await.unwrap();
        thread.join().unwrap();
        assert_eq!(read.occurrences[0].remote_id.as_deref(), Some("owned-a"));
    }

    #[test]
    fn removal_payload_targets_an_occurrence_instead_of_every_matching_video() {
        let track = SourceTrack::unavailable(0);
        let occurrence = PlaylistOccurrence {
            index: 1,
            track: SourceTrack {
                id: "duplicate".into(),
                ..track
            },
            remote_id: Some("owned-set-id".into()),
        };
        let actions = YouTubeMusicProvider::removal_actions(&[occurrence]).unwrap();
        assert_eq!(
            actions,
            json!([{"action":"ACTION_REMOVE_VIDEO","removedVideoId":"duplicate","setVideoId":"owned-set-id"}])
        );
    }

    #[tokio::test]
    async fn structured_read_keeps_http_retry_guidance() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut request = [0u8; 8192];
            socket.read(&mut request).unwrap();
            write!(socket,"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 60\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        let error = provider.read_playlist("fixture").await.unwrap_err();
        thread.join().unwrap();
        assert_eq!(error.kind, ProviderErrorKind::RateLimited);
        assert_eq!(error.retry_after_ms, Some(60_000));
        assert_eq!(error.certainty, SendCertainty::NotSent);
    }

    #[tokio::test]
    async fn session_canary_preserves_typed_rate_limit_guidance() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut request = [0u8; 8192];
            socket.read(&mut request).unwrap();
            write!(socket,"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 60\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        let error = provider.check_session().await.unwrap_err();
        thread.join().unwrap();
        assert_eq!(error.kind, ProviderErrorKind::RateLimited);
        assert_eq!(error.retry_after_ms, Some(60_000));
    }

    #[test]
    fn test_parse_duration_to_ms() {
        assert_eq!(YouTubeMusicProvider::parse_duration_to_ms("3:45"), 225_000);
        assert_eq!(YouTubeMusicProvider::parse_duration_to_ms("0:30"), 30_000);
        assert_eq!(
            YouTubeMusicProvider::parse_duration_to_ms("1:02:15"),
            3_735_000
        );
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
            if let Ok(Some(token)) =
                crate::auth::keyring_store::retrieve_credential("ytmusic", &conn)
            {
                crate::diagnostics::write_error(&format!(
                    "FOUND YTMUSIC TOKEN (len={})",
                    token.len()
                ));
                let provider = YouTubeMusicProvider::new(token);

                let playlists = provider
                    .list_playlists()
                    .await
                    .expect("Failed to list playlists");
                crate::diagnostics::write_error(&format!("FOUND {} PLAYLISTS", playlists.len()));
                assert!(!playlists.is_empty(), "Expected at least 1 playlist");
            } else {
                crate::diagnostics::write_error(&format!("NO YTMUSIC CREDENTIAL IN DB!"));
            }
        }
    }
}
