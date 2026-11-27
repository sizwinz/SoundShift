use crate::auth::sapisid::generate_youtube_auth_header;
use crate::models::{Playlist, SourceTrack};
use crate::providers::contracts::{
    MutationOutcome, OccurrenceOutcome, OccurrenceStatus, PlaylistOccurrence, PlaylistRead,
};
use crate::providers::error::{ProviderError, ProviderErrorKind, SendCertainty};
use crate::providers::traits::MusicProvider;
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, COOKIE, USER_AGENT};
use serde_json::{json, Value};

#[derive(Clone, Default)]
pub struct YouTubeMusicSessionCache {
    visitor_data: std::sync::Arc<std::sync::Mutex<Option<(String, String)>>>,
}

impl YouTubeMusicSessionCache {
    /// Clears any cached YouTube Music visitor context.
    pub fn clear(&self) {
        if let Ok(mut guard) = self.visitor_data.lock() {
            *guard = None;
        }
    }

    fn visitor_for(&self, cookie_str: &str) -> Option<String> {
        self.visitor_data
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .filter(|(cached_cookie, _)| cached_cookie == cookie_str)
            .map(|(_, visitor)| visitor)
    }

    fn set_visitor(&self, cookie_str: &str, visitor: String) {
        if let Ok(mut guard) = self.visitor_data.lock() {
            *guard = Some((cookie_str.to_string(), visitor));
        }
    }
}

#[derive(Clone)]
pub struct YouTubeMusicProvider {
    client: reqwest::Client,
    cookie_str: String,
    sapisid: String,
    visitor_data: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    session_cache: YouTubeMusicSessionCache,
    #[cfg(test)]
    edit_url: Option<String>,
}

impl YouTubeMusicProvider {
    pub fn new(token: String) -> Self {
        Self::with_session_cache(token, YouTubeMusicSessionCache::default())
    }

    /// Builds a YouTube Music provider backed by a shared visitor context cache.
    pub fn with_session_cache(token: String, session_cache: YouTubeMusicSessionCache) -> Self {
        let (cookie_str, sapisid) = if token.contains('=') {
            let find_cookie = |target: &str| {
                token.split(';').find_map(|part| {
                    let mut kv = part.trim().splitn(2, '=');
                    let k = kv.next()?.trim();
                    let v = kv.next()?.trim();
                    if k == target && !v.is_empty() {
                        Some(v.to_string())
                    } else {
                        None
                    }
                })
            };
            let extracted = find_cookie("SAPISID")
                .or_else(|| find_cookie("__Secure-3PAPISID"))
                .or_else(|| find_cookie("__Secure-1PAPISID"))
                .unwrap_or_else(|| token.clone());
            (token, extracted)
        } else {
            let cookie_str = format!("SAPISID={}; __Secure-3PAPISID={}", token, token);
            let sapisid = token;
            (cookie_str, sapisid)
        };

        let cached_visitor = session_cache.visitor_for(&cookie_str);
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
            visitor_data: std::sync::Arc::new(std::sync::Mutex::new(cached_visitor)),
            session_cache,
            #[cfg(test)]
            edit_url: None,
        }
    }

    fn capture_visitor_data(&self, value: &Value) {
        if let Some(visitor) = value
            .pointer("/responseContext/visitorData")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            if let Ok(mut guard) = self.visitor_data.lock() {
                *guard = Some(visitor.to_string());
            }
            self.session_cache
                .set_visitor(&self.cookie_str, visitor.to_string());
        }
    }

    async fn fetch_playlist(
        &self,
        playlist_id: &str,
        tx: Option<tokio::sync::mpsc::Sender<Vec<SourceTrack>>>,
    ) -> Result<PlaylistRead, ProviderError> {
        let browse_id = if playlist_id == "FEmusic_liked_videos" {
            "FEmusic_liked_videos".to_string()
        } else if playlist_id == "LM" || playlist_id == "VLLM" {
            "VLLM".to_string()
        } else if playlist_id.starts_with("VL") {
            playlist_id.to_string()
        } else {
            format!("VL{}", playlist_id)
        };

        let body = json!({
            "context": self.request_context(),
            "browseId": browse_id
        });

        let endpoint = "https://music.youtube.com/youtubei/v1/browse";
        #[cfg(test)]
        let endpoint = self.edit_url.as_deref().unwrap_or(endpoint);
        let json = self
            .post_json_read(
                "reading playlist",
                endpoint,
                &body,
                "Invalid playlist response schema",
            )
            .await?;

        let mut all_tracks = Vec::new();
        let mut remote_ids = Vec::new();
        let mut continuation_token: Option<String>;

        // Parse initial batch - support both twoColumnBrowseResultsRenderer (desktop web)
        // and singleColumnBrowseResultsRenderer (mobile/tablet layout)
        let sections = json
            .pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents")
            .or_else(|| json.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents"))
            .and_then(|v| v.as_array());

        let shelf = sections.and_then(|arr| {
            arr.iter().find_map(|section| {
                section
                    .get("musicPlaylistShelfRenderer")
                    .or_else(|| section.get("musicShelfRenderer"))
            })
        });

        let shelf_obj = shelf.ok_or_else(|| {
            ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::NotSent,
                "Playlist schema mismatch; incomplete read",
            )
        })?;
        {
            let empty_items: Vec<Value> = Vec::new();
            let items: &[Value] = match shelf_obj.get("contents") {
                None | Some(Value::Null) => &empty_items,
                Some(Value::Array(arr)) => arr.as_slice(),
                Some(_) => {
                    return Err(ProviderError::new(
                        ProviderErrorKind::MalformedResponse,
                        SendCertainty::NotSent,
                        "Playlist item schema mismatch; incomplete read",
                    ));
                }
            };
            {
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
                "context": self.request_context(),
            });

            #[cfg(test)]
            let url = self.edit_url.clone().unwrap_or(url);
            let cont_json = self
                .post_json_read(
                    "reading playlist tracks",
                    &url,
                    &cont_body,
                    "Invalid continuation response schema",
                )
                .await?;

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

    fn client_version() -> String {
        format!("1.{}.01.00", chrono::Utc::now().format("%Y%m%d"))
    }

    fn build_headers(&self) -> Result<HeaderMap, String> {
        let mut headers = HeaderMap::new();
        let origin = "https://music.youtube.com";
        let auth_val = generate_youtube_auth_header(&self.cookie_str, &self.sapisid, origin);
        let client_ver = Self::client_version();

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
        headers.insert("X-YouTube-Client-Name", HeaderValue::from_static("67"));
        if let Ok(ver_hdr) = HeaderValue::from_str(&client_ver) {
            headers.insert("X-YouTube-Client-Version", ver_hdr);
        }
        if let Some(visitor) = self
            .visitor_data
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
        {
            if let Ok(visitor_hdr) = HeaderValue::from_str(&visitor) {
                headers.insert("X-Goog-Visitor-Id", visitor_hdr);
            }
        }

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
                "clientVersion": Self::client_version(),
                "hl": "en",
                "gl": "US"
            },
            "user": {
                "lockedSafetyMode": false
            }
        })
    }

    fn request_context(&self) -> Value {
        let mut ctx = Self::client_context();
        let visitor = self
            .visitor_data
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .or_else(|| self.session_cache.visitor_for(&self.cookie_str));
        if let Some(visitor) = visitor {
            ctx["client"]["visitorData"] = json!(visitor);
        }
        ctx
    }

    async fn post_json_read(
        &self,
        action: &str,
        endpoint: &str,
        body: &Value,
        invalid_json_msg: &str,
    ) -> Result<Value, ProviderError> {
        let mut attempt = 0u32;
        loop {
            let headers = self.build_headers().map_err(ProviderError::before_send)?;
            let response = self
                .client
                .post(endpoint)
                .headers(headers)
                .json(body)
                .send()
                .await
                .map_err(ProviderError::read_transport)?;
            if response.status().is_success() {
                let value = response.json::<Value>().await.map_err(|_| {
                    ProviderError::new(
                        ProviderErrorKind::MalformedResponse,
                        SendCertainty::NotSent,
                        invalid_json_msg,
                    )
                })?;
                self.capture_visitor_data(&value);
                return Ok(value);
            }
            let status = response.status().as_u16();
            let err =
                ProviderError::read_http_for("YouTube Music", action, status, response.headers());
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

    fn extract_innertube_error(body: &str) -> Option<String> {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Ok(json) = serde_json::from_str::<Value>(trimmed) {
            let status = json.pointer("/error/status").and_then(|v| v.as_str());
            let message = json.pointer("/error/message").and_then(|v| v.as_str());
            match (status, message) {
                (Some(s), Some(m)) => return Some(format!("{}: {}", s, m)),
                (None, Some(m)) => return Some(m.to_string()),
                (Some(s), None) => return Some(s.to_string()),
                _ => {}
            }
        }
        let snippet: String = trimmed.chars().take(180).collect();
        Some(snippet)
    }

    fn find_playlist_id(value: &Value) -> Option<String> {
        if let Some(id) = value.get("playlistId").and_then(|v| v.as_str()) {
            if !id.trim().is_empty() {
                return Some(id.to_string());
            }
        }
        if let Some(id) = value.get("createdPlaylistId").and_then(|v| v.as_str()) {
            if !id.trim().is_empty() {
                return Some(id.to_string());
            }
        }
        if let Some(actions) = value.get("actions").and_then(|v| v.as_array()) {
            for action in actions {
                if let Some(found) = Self::find_playlist_id(action) {
                    return Some(found);
                }
            }
        }
        if let Some(obj) = value.as_object() {
            for child in obj.values() {
                if child.is_object() || child.is_array() {
                    if let Some(found) = Self::find_playlist_id(child) {
                        return Some(found);
                    }
                }
            }
        }
        None
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

    fn is_signed_out_browse_response(value: &Value) -> bool {
        if let Some(services) = value
            .pointer("/responseContext/serviceTrackingParams")
            .and_then(|v| v.as_array())
        {
            for service in services {
                if let Some(params) = service.get("params").and_then(|v| v.as_array()) {
                    for param in params {
                        let key = param.get("key").and_then(|v| v.as_str());
                        let val = param.get("value").and_then(|v| v.as_str());
                        if matches!(key, Some("logged_in" | "yt_li")) && val == Some("0") {
                            return true;
                        }
                    }
                }
            }
        }

        if let Some(sections) = value
            .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
            .and_then(|v| v.as_array())
        {
            for section in sections {
                if let Some(items) = section
                    .pointer("/itemSectionRenderer/contents")
                    .and_then(|v| v.as_array())
                {
                    for item in items {
                        if item
                            .pointer("/messageRenderer/button/buttonRenderer/navigationEndpoint/signInEndpoint")
                            .is_some()
                        {
                            return true;
                        }
                    }
                }
            }
        }

        false
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
        let value = self
            .post_json_read(
                "checking playlist editability",
                endpoint,
                &json!({"context":self.request_context(),"browseId":if playlist_id.starts_with("VL"){playlist_id.to_string()}else{format!("VL{playlist_id}")}}),
                "YouTube playlist permissions response is malformed",
            )
            .await?;
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
        let endpoint = "https://music.youtube.com/youtubei/v1/browse";
        #[cfg(test)]
        let endpoint = self.edit_url.as_deref().unwrap_or(endpoint);
        let value = self
            .post_json_read(
                "checking session",
                endpoint,
                &json!({"context":self.request_context(),"browseId":"FEmusic_liked_playlists"}),
                "Session canary returned invalid JSON",
            )
            .await?;
        self.capture_visitor_data(&value);
        if Self::is_signed_out_browse_response(&value) {
            return Err(ProviderError::new(
                ProviderErrorKind::Authentication,
                SendCertainty::NotSent,
                "YouTube Music session is signed out or expired (401); please reconnect YouTube Music",
            ));
        }
        if value.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents").and_then(|v|v.as_array()).is_none() {
            return Err(ProviderError::new(ProviderErrorKind::MalformedResponse,SendCertainty::NotSent,"Session canary schema mismatch; reconnect or review provider availability"));
        }
        Ok(())
    }

    async fn list_playlists(&self) -> Result<Vec<Playlist>, String> {
        let headers = self.build_headers()?;
        let body = json!({
            "context": self.request_context(),
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
        self.search_track_resilient(query)
            .await
            .map_err(|e| e.to_string())
    }

    async fn search_track_resilient(&self, query: &str) -> Result<Vec<SourceTrack>, ProviderError> {
        let headers = self.build_headers().map_err(ProviderError::before_send)?;
        let search_endpoint = "https://music.youtube.com/youtubei/v1/search";
        #[cfg(test)]
        let search_endpoint = self.edit_url.as_deref().unwrap_or(search_endpoint);

        // Primary search: Songs filter per D-05
        let body = json!({
            "context": self.request_context(),
            "query": query,
            "params": "Eg-KAQwIARAAGAAgACgAMABqChAMEAMQBBAFEAo%3D"
        });

        let res = self
            .client
            .post(search_endpoint)
            .headers(headers.clone())
            .json(&body)
            .send()
            .await
            .map_err(ProviderError::read_transport)?;

        if !res.status().is_success() {
            return Err(ProviderError::read_http_for(
                "YouTube Music",
                "searching tracks",
                res.status().as_u16(),
                res.headers(),
            ));
        }

        let mut results = Vec::new();
        if let Ok(json) = res.json::<Value>().await {
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

        // Secondary fallback: Video search if songs filter yielded no candidate per D-05
        if results.is_empty() {
            let video_body = json!({
                "context": self.request_context(),
                "query": query,
                "params": "Eg-KAQwIAhABGAAgACgAMABqChAMEAMQBBAFEAo%3D"
            });

            let video_res = self
                .client
                .post(search_endpoint)
                .headers(headers)
                .json(&video_body)
                .send()
                .await
                .map_err(ProviderError::read_transport)?;

            if !video_res.status().is_success() {
                return Err(ProviderError::read_http_for(
                    "YouTube Music",
                    "searching tracks",
                    video_res.status().as_u16(),
                    video_res.headers(),
                ));
            }

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

        Ok(results)
    }

    async fn fetch_track_by_id(
        &self,
        video_id: &str,
    ) -> Result<Option<SourceTrack>, ProviderError> {
        let headers = self.build_headers().map_err(ProviderError::before_send)?;
        let endpoint = "https://music.youtube.com/youtubei/v1/next";
        #[cfg(test)]
        let endpoint = self.edit_url.as_deref().unwrap_or(endpoint);
        let res = self
            .client
            .post(endpoint)
            .headers(headers)
            .json(&json!({
                "context": self.request_context(),
                "videoId": video_id
            }))
            .send()
            .await
            .map_err(ProviderError::read_transport)?;
        if !res.status().is_success() {
            return Err(ProviderError::read_http_for(
                "YouTube Music",
                "loading track metadata",
                res.status().as_u16(),
                res.headers(),
            ));
        }
        let json: Value = match res.json().await {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        let contents = json
            .pointer("/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs/0/tabRenderer/content/musicQueueRenderer/content/playlistPanelRenderer/contents")
            .and_then(|v| v.as_array());
        if let Some(items) = contents {
            for item in items {
                let renderer = &item["playlistPanelVideoRenderer"];
                if renderer.is_null() {
                    continue;
                }
                let vid = renderer["videoId"].as_str().unwrap_or("");
                if vid != video_id {
                    continue;
                }
                let title = renderer["title"]["runs"][0]["text"]
                    .as_str()
                    .unwrap_or("Unknown")
                    .to_string();
                let mut artists = Vec::new();
                if let Some(runs) = renderer["longBylineText"]["runs"].as_array() {
                    for run in runs {
                        let txt = run["text"].as_str().unwrap_or("").trim();
                        if txt == "•" {
                            break;
                        }
                        if !txt.is_empty() && txt != "," && txt != "&" {
                            artists.push(txt.to_string());
                        }
                    }
                }
                if artists.is_empty() {
                    if let Some(byline) = renderer["shortBylineText"]["runs"][0]["text"].as_str() {
                        artists.push(byline.to_string());
                    }
                }
                let duration_ms = renderer["lengthText"]["runs"][0]["text"]
                    .as_str()
                    .map(Self::parse_duration_to_ms)
                    .unwrap_or(0);
                let thumbnail_url = renderer["thumbnail"]["thumbnails"]
                    .as_array()
                    .and_then(|arr| arr.last())
                    .and_then(|t| t["url"].as_str())
                    .map(|s| s.to_string());
                return Ok(Some(SourceTrack {
                    id: vid.to_string(),
                    title,
                    artists,
                    album: None,
                    duration_ms,
                    isrc: None,
                    is_explicit: false,
                    is_playable: true,
                    preview_url: None,
                    thumbnail_url,
                }));
            }
        }
        Ok(None)
    }

    async fn create_playlist_checked(
        &self,
        title: &str,
        description: Option<&str>,
    ) -> Result<String, ProviderError> {
        let headers = self.build_headers().map_err(ProviderError::before_send)?;
        let stripped = title.replace(['<', '>'], "");
        let clean_title = if stripped.trim().is_empty() {
            "Transferred Playlist"
        } else {
            stripped.trim()
        };
        let body = json!({
            "context": self.request_context(),
            "title": clean_title,
            "description": description.unwrap_or(""),
            "privacyStatus": "PRIVATE"
        });

        let endpoint = "https://music.youtube.com/youtubei/v1/playlist/create";
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
            let status = res.status().as_u16();
            let res_headers = res.headers().clone();
            let body_text = res.text().await.unwrap_or_default();
            let detail = Self::extract_innertube_error(&body_text);
            return Err(ProviderError::http_with_detail(
                status,
                &res_headers,
                detail.as_deref(),
            ));
        }

        let json: Value = res.json().await.map_err(|_| {
            ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::MayHaveApplied,
                "Invalid playlist creation response; check YouTube Music before retrying",
            )
        })?;

        if let Some(err_msg) = json.pointer("/error/message").and_then(|v| v.as_str()) {
            return Err(ProviderError::new(
                crate::providers::error::classify_error(err_msg),
                SendCertainty::Rejected,
                &format!("YouTube Music rejected playlist creation: {}", err_msg),
            ));
        }

        Self::find_playlist_id(&json).ok_or_else(|| {
            ProviderError::new(
                ProviderErrorKind::MalformedResponse,
                SendCertainty::MayHaveApplied,
                "YouTube Music response omitted playlistId; check provider before retrying",
            )
        })
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
            return Err("YouTube Music mutation outcome uncertain: missing occurrence evidence; reconcile before retry".into());
        }
        Ok(())
    }

    async fn add_occurrences_with_options(
        &self,
        playlist_id: &str,
        track_ids: &[String],
        skip_duplicates: bool,
    ) -> Result<MutationOutcome, ProviderError> {
        let headers = self.build_headers().map_err(ProviderError::before_send)?;
        let clean_id = playlist_id.trim_start_matches("VL");

        let actions: Vec<Value> = track_ids
            .iter()
            .map(|id| {
                if skip_duplicates {
                    json!({
                        "action": "ACTION_ADD_VIDEO",
                        "addedVideoId": id,
                        "dedupeOption": "DEDUPE_OPTION_SKIP"
                    })
                } else {
                    json!({
                        "action": "ACTION_ADD_VIDEO",
                        "addedVideoId": id
                    })
                }
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

    async fn add_occurrences(
        &self,
        playlist_id: &str,
        track_ids: &[String],
    ) -> Result<MutationOutcome, ProviderError> {
        self.add_occurrences_with_options(playlist_id, track_ids, false)
            .await
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

    #[tokio::test]
    async fn check_session_rejects_logged_out_youtube_music_browse_response() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut request = [0u8; 8192];
            socket.read(&mut request).unwrap();
            let body = json!({
                "responseContext": {
                    "visitorData": "CgsxNWNxN2pRbFV1WQ==",
                    "serviceTrackingParams": [
                        {
                            "service": "GFEEDBACK",
                            "params": [{"key": "logged_in", "value": "0"}]
                        },
                        {
                            "service": "CSI",
                            "params": [{"key": "yt_li", "value": "0"}]
                        }
                    ]
                },
                "contents": {
                    "singleColumnBrowseResultsRenderer": {
                        "tabs": [{
                            "tabRenderer": {
                                "content": {
                                    "sectionListRenderer": {
                                        "contents": [{
                                            "itemSectionRenderer": {
                                                "contents": [{
                                                    "messageRenderer": {
                                                        "button": {
                                                            "buttonRenderer": {
                                                                "navigationEndpoint": {"signInEndpoint": {}}
                                                            }
                                                        }
                                                    }
                                                }]
                                            }
                                        }]
                                    }
                                }
                            }
                        }]
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
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        let error = provider.check_session().await.unwrap_err();
        thread.join().unwrap();
        assert_eq!(error.kind, ProviderErrorKind::Authentication);
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
    async fn search_track_resilient_propagates_http_429_instead_of_empty_ok() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut request = [0u8; 8192];
            socket.read(&mut request).unwrap();
            write!(socket, "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 12\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        let err = provider
            .search_track_resilient("Starboy")
            .await
            .unwrap_err();
        thread.join().unwrap();
        assert_eq!(err.kind, ProviderErrorKind::RateLimited);
        assert_eq!(err.retry_after_ms, Some(12_000));
    }

    #[tokio::test]
    async fn add_occurrences_with_options_parameterizes_dedupe_option() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let mut bodies = Vec::new();
            for _ in 0..2 {
                let (mut socket, _) = server.accept().unwrap();
                let mut bytes = [0u8; 8192];
                let size = socket.read(&mut bytes).unwrap();
                let req = String::from_utf8_lossy(&bytes[..size]).to_string();
                bodies.push(req.split("\r\n\r\n").nth(1).unwrap().to_string());
                let resp = r#"{"status":"STATUS_SUCCEEDED","playlistEditResults":[{"playlistEditVideoAddedResultData":{"videoId":"a","setVideoId":"s1"}}]}"#;
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", resp.len(), resp).unwrap();
            }
            bodies
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        provider
            .add_occurrences_with_options("fixture", &["a".into()], false)
            .await
            .unwrap();
        provider
            .add_occurrences_with_options("fixture", &["a".into()], true)
            .await
            .unwrap();
        let bodies = thread.join().unwrap();
        let first: Value = serde_json::from_str(&bodies[0]).unwrap();
        let second: Value = serde_json::from_str(&bodies[1]).unwrap();
        assert!(first["actions"][0]["dedupeOption"].is_null());
        assert_eq!(second["actions"][0]["dedupeOption"], "DEDUPE_OPTION_SKIP");
    }

    #[tokio::test]
    async fn fetch_playlist_supports_liked_videos_without_vl_prefix() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let mut captured = String::new();
            for i in 0..2 {
                let (mut socket, _) = server.accept().unwrap();
                let mut bytes = [0u8; 8192];
                let size = socket.read(&mut bytes).unwrap();
                let req = String::from_utf8_lossy(&bytes[..size]).to_string();
                if i == 0 {
                    captured = req.split("\r\n\r\n").nth(1).unwrap().to_string();
                }
                let body = json!({"contents":{"twoColumnBrowseResultsRenderer":{"secondaryContents":{"sectionListRenderer":{"contents":[{"musicShelfRenderer":{"contents":[]}}]}}}}}).to_string();
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
            captured
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        provider
            .read_playlist("FEmusic_liked_videos")
            .await
            .unwrap();
        let body: Value = serde_json::from_str(&thread.join().unwrap()).unwrap();
        assert_eq!(body["browseId"], "FEmusic_liked_videos");
    }

    #[tokio::test]
    async fn empty_playlist_shelf_without_contents_reads_as_zero_tracks() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut socket, _) = server.accept().unwrap();
                let mut bytes = [0u8; 8192];
                socket.read(&mut bytes).unwrap();
                let body = json!({
                    "contents": {
                        "twoColumnBrowseResultsRenderer": {
                            "secondaryContents": {
                                "sectionListRenderer": {
                                    "contents": [{
                                        "musicPlaylistShelfRenderer": {
                                            "playlistId": "PL_empty",
                                            "trackingParams": "abc"
                                        }
                                    }]
                                }
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
            }
        });
        let mut provider = YouTubeMusicProvider::new("fixture".into());
        provider.edit_url = Some(format!("http://{address}"));
        let read = provider.read_playlist("PL_empty").await.unwrap();
        thread.join().unwrap();
        assert!(read.complete);
        assert!(read.occurrences.is_empty());
    }

    #[tokio::test]
    async fn create_playlist_checked_sends_client_headers_sanitizes_title_and_captures_http_detail()
    {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for idx in 0..2 {
                let (mut socket, _) = server.accept().unwrap();
                let mut bytes = [0u8; 8192];
                let size = socket.read(&mut bytes).unwrap();
                let req = String::from_utf8_lossy(&bytes[..size]).to_string();
                requests.push(req);
                if idx == 0 {
                    let body = json!({"error":{"code":400,"status":"INVALID_ARGUMENT","message":"Invalid argument"}}).to_string();
                    write!(socket, "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
                } else {
                    let body = json!({"actions":[{"createPlaylistServiceEndpoint":{"playlistId":"PL_nested_123"}}]}).to_string();
                    write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
                }
            }
            requests
        });

        let mut provider = YouTubeMusicProvider::new(
            "SAPISID=base_secret; __Secure-1PAPISID=p1_secret; __Secure-3PAPISID=p3_secret".into(),
        );
        provider.edit_url = Some(format!("http://{address}"));

        let err = provider
            .create_playlist_checked("Infinite <3", Some("Desc"))
            .await
            .unwrap_err();
        assert_eq!(err.certainty, SendCertainty::Rejected);
        assert_eq!(err.http_status, Some(400));
        assert!(err.detail.contains("400"));
        assert!(err.detail.contains("Invalid argument"));

        let created_id = provider
            .create_playlist_checked("Infinite <3", Some("Desc"))
            .await
            .unwrap();
        assert_eq!(created_id, "PL_nested_123");

        let requests = thread.join().unwrap();
        let first_req_lower = requests[0].to_ascii_lowercase();
        assert!(first_req_lower.contains("x-youtube-client-name: 67"));
        assert!(first_req_lower.contains("x-youtube-client-version: 1."));
        assert!(requests[0].contains("SAPISIDHASH "));
        assert!(requests[0].contains("SAPISID1PHASH "));
        assert!(requests[0].contains("SAPISID3PHASH "));

        let body_str = requests[0].split("\r\n\r\n").nth(1).unwrap();
        let body_json: Value = serde_json::from_str(body_str).unwrap();
        assert_eq!(body_json["title"], "Infinite 3");
        assert!(body_json["context"]["user"].is_object());
    }

    #[tokio::test]
    async fn read_playlist_retries_short_429_and_attributes_ytmusic() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let mut count = 0;
            for idx in 0..3 {
                let (mut socket, _) = server.accept().unwrap();
                let mut bytes = [0u8; 8192];
                let _ = socket.read(&mut bytes);
                count += 1;
                if idx == 0 {
                    write!(
                        socket,
                        "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                } else {
                    let body = r#"{"contents":{"twoColumnBrowseResultsRenderer":{"secondaryContents":{"sectionListRenderer":{"contents":[{"musicPlaylistShelfRenderer":{"contents":[]}}]}}}}}"#;
                    write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .unwrap();
                }
            }
            count
        });
        let mut provider = YouTubeMusicProvider::new("SAPISID=123; __Secure-3PAPISID=123".into());
        provider.edit_url = Some(format!("http://{address}"));
        let read = provider.read_playlist("PL_empty").await.unwrap();
        assert!(read.complete);
        assert_eq!(thread.join().unwrap(), 3);
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
