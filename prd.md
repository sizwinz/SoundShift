# Product Requirements Document (PRD)

**Project Name:** SoundShift (Working Title)

**Status:** Draft / Ready for Implementation

**Target Release:** v0.1.0-alpha

**Platform:** Cross-Platform Desktop (Linux, macOS, Windows)

---

## 1. Problem Statement & Executive Summary

Commercial playlist migration tools (TuneMyMusic, Soundiiz) monetize basic data mobility by imposing track count limits (typically 500 tracks on free tiers), recurring subscription models, and closed matching algorithms that silently substitute incorrect song variants (e.g., live recordings, acoustics, or karaoke tracks). Existing open-source alternatives (such as SpotTransfer or CLI scripts) often require users to manually extract sensitive authentication headers via browser DevTools, run local Python environments, or navigate restrictive cloud API rate limits that fail on large libraries.

**SoundShift** is an open-source, local-first desktop application engineered to migrate, sync, and back up music playlists across streaming services (launching with Spotify and YouTube Music) with zero API keys required from the end user, deterministic matching heuristics, a visual staging "diff" inspection interface, and 1-click snapshot rollback.

---

## 2. Goals & Non-Goals

### Core Objectives

* **Zero DevTools Onboarding:** End users must never manually inspect network tabs or copy-paste cookie strings. Authentication occurs inside an embedded, isolated native WebView.
* **Deterministic Matching Accuracy ($\ge 98\%$ target):** Prevent version drift using International Standard Recording Codes (ISRC), strict duration delta bounds ($\le 4\text{s}$), and metadata normalization before falling back to manual disambiguation.
* **Inspectable Staging ("Git Diff for Playlists"):** No mutations occur on the destination platform until the user reviews, audits, and confirms candidate matches.
* **Zero Operational Cost & Uncapped Migrations:** 100% client-side execution running on the user's network connection and IP, eliminating shared cloud rate-limiting, serverless execution timeouts, and hosting expenses.
* **Library Safety:** Every transfer operation creates an immutable local snapshot enabling complete undo/rollback capabilities.

### Non-Goals

* **Zero Cloud Synchronization:** The application will not run a hosted multi-tenant cloud backend or sync daemon. Background scheduled syncing will only execute locally if the user enables a system-tray daemon.
* **Audio Downloading/Ripping:** The application is strictly a metadata migrator; it will not download, encode, or store audio files locally (no `yt-dlp` audio ripping).

---

## 3. Architecture & Tech Stack

```
┌────────────────────────────────────────────────────────┐
│               SoundShift Desktop Shell                 │
│               (Tauri v2 + Rust Core)                   │
├──────────────────────────┬─────────────────────────────┤
│ Frontend Presentation    │ Core Rust Subsystems        │
│ • React 19 + TypeScript  │ • Embedded WebView Auth Trap│
│ • Tailwind CSS + shadcn  │ • Async Job Pipeline (Tokio)│
│ • TanStack Virtual List  │ • SQLite Match & Diff Store │
│ • Lucide Icons           │ • InnerTube / Spotify Engine│
└──────────────────────────┴─────────────────────────────┘

```

| Layer | Technology | Rationale |
| --- | --- | --- |
| **Shell & Runtime** | Tauri v2 (Rust) | Sub-15MB installer footprint, minimal RAM usage compared to Electron, direct control over OS WebViews and network headers. |
| **Frontend Framework** | React 19 + TypeScript + Vite | Standard reactive UI, strong typing across platform drivers. |
| **Styling & Components** | Tailwind CSS + `shadcn/ui` | True-black AMOLED/dark-first aesthetic, accessible primitives (modals, drawers, tooltips). |
| **List Virtualization** | `@tanstack/react-virtual` | Smooth 60 FPS scrolling on playlists containing 10,000+ tracks. |
| **Embedded Database** | SQLite via `rusqlite` | Local caching of ISRC mappings, transfer history, and snapshot rollbacks. |
| **Networking** | `reqwest` (HTTP/2) | Connection pooling, custom header spoofing, exponential backoff handling. |

---

## 4. System Components & Functional Requirements

### 4.1. Authentication Subsystem (Zero-DevTools Capture)

* **FR-1.1 (Embedded Login Modal):** When a user connects YouTube Music or Spotify, the app launches an isolated, dedicated Webview pointing directly to the official login URL (`[https://music.youtube.com](https://music.youtube.com)` or `[https://accounts.spotify.com](https://accounts.spotify.com)`).
* **FR-1.2 (Cookie & Token Interception):**
* *YouTube Music:* The Rust runtime monitors HTTP request headers within the WebView session, intercepting `cookie` values containing `SAPISID`, `__Secure-3PAPISID`, and `HSID`. It computes the required `SAPISIDHASH` authorization header natively.
* *Spotify:* The WebView session captures the `sp_dc` cookie or extracts the ephemeral access token directly from `[open.spotify.com/get_access_token](https://open.spotify.com/get_access_token)`.


* **FR-1.3 (Secure Credential Storage):** Extracted tokens and cookies are saved in the platform-native secure credential store (OS Keyring via `keyring-rs`) or encrypted with AES-256-GCM in the local SQLite database.

---

### 4.2. Provider Driver Abstraction

All streaming service integrations must implement a unified Rust trait / TypeScript contract:

```typescript
export interface SourceTrack {
  id: string;
  title: string;
  artists: string[];
  album: string;
  durationMs: number;
  isrc?: string;
  previewUrl?: string;
  thumbnailUrl?: string;
}

export interface MatchCandidate {
  id: string;
  title: string;
  artists: string[];
  durationMs: number;
  confidenceScore: number; // 0.0 to 1.0
  matchMethod: 'ISRC' | 'EXACT_METRIC' | 'FUZZY_CONFIRMED' | 'MANUAL';
  previewUrl?: string;
}

export interface MusicProvider {
  id: string;
  name: string;
  checkAuth(): Promise<boolean>;
  getPlaylists(): Promise<PlaylistSummary[]>;
  getPlaylistTracks(playlistId: string): Promise<SourceTrack[]>;
  createPlaylist(name: string, description: string): Promise<string>;
  searchTrack(track: SourceTrack): Promise<MatchCandidate[]>;
  addTracksToPlaylist(playlistId: string, trackIds: string[]): Promise<void>;
  removeTracksFromPlaylist(playlistId: string, trackIds: string[]): Promise<void>;
}

```

---

### 4.3. 4-Stage Track Matching Engine

To achieve the $\ge 98\%$ target without human intervention on standard studio recordings, search queries must run through a four-stage pipeline:

```
[Incoming Track]
      │
      ▼
┌───────────────────────────┐
│ Pass 1: Local Cache Match │ ──(Found in SQLite)──► [Confidence: 1.0 | Accept]
└─────────────┬─────────────┘
              │ (Miss)
              ▼
┌───────────────────────────┐
│ Pass 2: Direct ISRC Query │ ──(Exact ISRC Match)──► [Confidence: 1.0 | Accept]
└─────────────┬─────────────┘
              │ (Miss)
              ▼
┌───────────────────────────┐
│ Pass 3: Fuzzy Duration-   │
│         Anchored Search   │ ──(Score ≥ 0.88 AND ΔTime ≤ 4s)──► [Confidence: High | Accept]
└─────────────┬─────────────┘
              │ (Fails criteria)
              ▼
┌───────────────────────────┐
│ Pass 4: Disambiguation    │ ──► [Flag as AMBER | Route to Staging Review]
└───────────────────────────┘

```

#### Heuristic Specifications

1. **Metadata Normalization:** Prior to executing target platform searches, strip all noise tokens using regex:
```regex
(?i)(\(feat\..*?\)|\[feat\..*?\]|\(remastered.*?\)|\[remastered.*?\]|\(official.*?\)|\[official.*?\]|- single|- acoustic)

```


2. **The Duration Anchor Rule:** For any text-based candidate match, compute the absolute duration delta:

$$\Delta t = \vert{}\text{duration}_{\text{source}} - \text{duration}_{\text{target}}\vert{}$$


* If $\Delta t \le 4\text{s}$ and normalized Levenshtein score $\ge 0.85$, auto-mark as **Matched (Green)**.
* If $4\text{s} < \Delta t \le 15\text{s}$, or Levenshtein score is between $0.65$ and $0.85$, auto-mark as **Needs Review (Amber)**.
* If $\Delta t > 15\text{s}$, reject candidate to eliminate live versions, extended mixes, and interview clips.



---

### 4.4. Staging & Diff Interface ("Git for Music")

* **FR-4.1 (Pre-Execution Staging):** The application must never write directly to the target platform until the matching phase is complete and the user has viewed the staging screen.
* **FR-4.2 (Categorized Diff Rows):**
* `EXACT` (Green): ISRC match or high-confidence score. Auto-checked for transfer.
* `AMBIGUOUS` (Amber): Multiple candidates found, or duration variance between 4–15s. Requires explicit user sign-off.
* `NOT_FOUND` (Red): Zero candidates returned. Unchecked by default; option to paste a manual search query or target URL.


* **FR-4.3 (Disambiguation Drawer):** Clicking an Amber row opens an inline split drawer:
* Left: Source track details (album, artist, duration, artwork).
* Right: Top 3 candidate matches ranked by confidence.
* Preview buttons for both source and candidate using 30-second audio stream segments.


* **FR-4.4 (Batch Overrides):** Ability to "Accept All Recommendations" or "Skip All Unresolved".

---

### 4.5. Batch Execution, Resilience & Rollback

* **FR-5.1 (Rate-Limiter & Chunking Engine):** Requests to streaming APIs must run via an asynchronous worker pool limited to $N$ concurrent tasks (default: 4 concurrent calls) with exponential backoff on HTTP 429 status codes (`base_delay: 1.5s`, `max_retries: 5`).
* **FR-5.2 (Local Pre-Mutation Snapshot):** Before inserting any tracks into a new or existing playlist, SoundShift writes a complete JSON snapshot to SQLite:
```json
{
  "snapshot_id": "snp_9a8b7c",
  "timestamp": 1790870400,
  "platform": "ytmusic",
  "playlist_id": "PLabc12345",
  "tracks_added": ["video_id_1", "video_id_2"],
  "previous_track_order": []
}

```


* **FR-5.3 (1-Click Rollback):** Users can navigate to "Transfer History" and click "Rollback". The app issues sequential deletion requests for every track ID recorded in `tracks_added`.

---

## 5. Local Database Schema (SQLite)

```sql
CREATE TABLE IF NOT EXISTS service_sessions (
    service_id TEXT PRIMARY KEY, -- 'spotify' | 'ytmusic'
    auth_data TEXT NOT NULL,     -- Encrypted session payload
    expires_at INTEGER,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS track_match_cache (
    source_service TEXT NOT NULL,
    source_track_id TEXT NOT NULL,
    target_service TEXT NOT NULL,
    target_track_id TEXT NOT NULL,
    match_method TEXT NOT NULL,
    confidence REAL NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (source_service, source_track_id, target_service)
);

CREATE TABLE IF NOT EXISTS transfer_jobs (
    job_id TEXT PRIMARY KEY,
    source_service TEXT NOT NULL,
    target_service TEXT NOT NULL,
    source_playlist_name TEXT NOT NULL,
    target_playlist_id TEXT NOT NULL,
    total_tracks INTEGER NOT NULL,
    matched_tracks INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS transfer_snapshots (
    snapshot_id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    mutation_payload TEXT NOT NULL, -- JSON serialization of added/modified tracks
    is_rolled_back INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL,
    FOREIGN KEY(job_id) REFERENCES transfer_jobs(job_id)
);

```

---

## 6. UI/UX Design System Specifications

* **Design Philosophy:** Minimalist, geometric, developer-tool aesthetic (dark mode default: `#09090b` zinc / `#000000` AMOLED; accent: warm amber/copper `#d97706` for actions and `#10b981` emerald for verified matches).
* **Information Density:** High information density. Compact track rows (40px row height), monospace typography for timecodes and ISRCs (`JetBrains Mono` or system mono), clear status badges.
* **Layout Structure:**
1. *Sidebar (64px collapsed / 200px expanded):* Transfers, History/Rollback, Connected Accounts, Settings.
2. *Main Panel:* Three-step breadcrumb workflow:
* `Step 1: Source & Target Selection`
* `Step 2: Matching & Diff Review`
* `Step 3: Transfer Progress & Telemetry`


3. *Live Progress Console:* Minimizable real-time log drawer streaming execution stats (e.g., `[22:15:04] Matched: "Resonance" via ISRC -> US-HR1-14-00124 (2ms)`).



---

## 7. Non-Functional Requirements

* **Performance:**
* Cold launch time: $\le 1.2\text{ seconds}$ on standard SSD hardware.
* Memory consumption: $\le 85\text{MB}$ active idle; $\le 190\text{MB}$ during active 5,000-track diff parsing.
* Virtualized list must maintain 60 FPS scrolling with 15,000 track entities.


* **Security & Privacy:**
* Zero telemetry, tracking, or network calls to external analytical servers.
* All tokens kept strictly on the user's local disk; network calls are strictly restricted to target platform endpoints (`api.spotify.com`, `music.youtube.com`).


* **Packaging:**
* Self-contained executables: `.AppImage` and `.tar.gz` for Linux, `.dmg` (Universal) for macOS, and `.msi` / standalone `.exe` for Windows.



---

## 8. Release Milestones & Implementation Phases

```
[Phase 1: Proof of Concept]
  ├── Tauri v2 scaffold + embedded Webview auth extraction
  ├── Spotify public metadata ingest (unauthenticated / embed token)
  └── YouTube Music InnerTube driver (authenticated via captured cookies)
      │
      ▼
[Phase 2: The Core Engine]
  ├── Multi-stage matching pipeline (ISRC + Duration delta filtering)
  ├── SQLite caching layer
  └── Headless CLI testing suite for match rate benchmarks
      │
      ▼
[Phase 3: The UI / Review Experience]
  ├── Staging "Git Diff" table + manual conflict resolution drawer
  ├── Audio preview integration
  └── Progress state machine with pause / resume / retry
      │
      ▼
[Phase 4: Safety & Production Polish]
  ├── Snapshot creation and 1-click Rollback execution
  ├── Native binary packaging (CI/CD via GitHub Actions)
  └── Driver extensions (Apple Music / TIDAL preliminary support)

```

---

## 9. Risk Matrix & Mitigations

| Risk | Impact | Probability | Engineering Mitigation |
| --- | --- | --- | --- |
| **YouTube Music InnerTube Changes** | High | Medium | Decouple the InnerTube endpoint definitions into a remote, auto-updating JSON schema that the app can fetch at launch without requiring a full desktop binary rebuild. |
| **Spotify Anti-Scraping / Token Expiry** | Medium | Medium | Maintain dual ingest pathways: (1) Official Spotify API client credentials for users who want to supply keys, and (2) Fallback to public embed web parser for public playlists. |
| **IP-Based Temporary Rate Limits** | High | Low/Medium | Implement dynamic backoff throttling. Add a user-facing concurrency slider (1 to 8 workers) to dial down speed on restrictive connections. |
| **Large Memory Footprint on 10k+ Tracks** | Medium | Low | Enforce pagination and cursor streaming from the database straight into `@tanstack/react-virtual`, never keeping 10,000 full song objects unindexed in JavaScript memory. |