# SoundShift

SoundShift is an open-source, local-first desktop application engineered to migrate, synchronize, and back up music playlists across streaming services (Spotify and YouTube Music).

Designed with zero subscription paywalls, zero track count caps, and zero developer API key requirements, SoundShift runs entirely on your local machine with native WebView authentication, deterministic multi-stage matching heuristics, an inspectable staging "diff" interface, and 1-click snapshot rollback.

---

## Key Features

- **Zero DevTools Onboarding:** Embedded native WebView sessions automatically capture and store authenticated session credentials into your operating system's native secure credential store (Windows Credential Locker, macOS Keychain, Linux Secret Service).
- **Deterministic 4-Stage Matching:**
  - **Pass 1 (ISRC Exact Match):** Canonical international recording codes guarantee exact audio recording matches.
  - **Pass 2 (Direct Metadata Search):** Normalized search query execution against destination catalog APIs.
  - **Pass 3 (Fuzzy Text & Duration Anchor):** Levenshtein and token-sorted similarity evaluation anchored by a strict duration delta bound (<= 4s for Exact Green; 4s to 15s for Ambiguous Amber).
  - **Pass 4 (Variant Rejection):** Automatic rejection of tracks with duration variance > 15s to eliminate unwanted live cuts, extended mixes, interviews, and acoustic re-recordings.
- **Inspectable Staging Diff ("Git Diff for Playlists"):** Review all proposed matches before making any changes to your library. Categorized into Exact (Green), Ambiguous (Amber), and Not Found (Red).
- **High-Performance Virtualization:** Sustained 60 FPS scrolling on 15,000+ track entities powered by `@tanstack/react-virtual`.
- **In-App Audio Preview:** 30-second HTML5 audio previews inside the Disambiguation Drawer to verify ambiguous matches before confirming.
- **Resilient Batch Transfer:** Optimized chunked atomic mutations with automatic exponential backoff on HTTP 429 and 409 responses, preserving exact playlist order.
- **Safety & 1-Click Rollback:** Transactional pre-mutation snapshots saved in local SQLite storage enable immediate 1-click undo for every transfer.
- **Strictly Local-First Privacy:** Zero third-party telemetry, zero cloud backends, zero external proxies. All data and credentials stay on your device.

---

## Tech Stack

| Layer | Technologies | Rationale |
| --- | --- | --- |
| **Desktop Shell** | Tauri v2 | Sub-15MB installer, <= 85MB idle RAM, direct OS WebView integration |
| **Core Systems Engine** | Rust 1.80+ (2021 Edition) | High-throughput asynchronous pipeline, native HTTP/2 handling via `reqwest`, SQLite via `rusqlite` |
| **Frontend UI** | React 19, TypeScript, Vite | Modern declarative rendering, strict compile-time types |
| **Styling & Components** | Tailwind CSS v4, Lucide Icons | Dark-first AMOLED design system, responsive layouts |
| **List Virtualization** | `@tanstack/react-virtual` v3 | Virtualized DOM sustaining 60 FPS on 15,000+ tracks |
| **Security** | OS Keyring (`keyring-rs`), AES-256-GCM | Encrypted local token vault |

---

## Architecture Overview

```
+-----------------------------------------------------------------+
|                    SoundShift Desktop Shell                     |
|                      (Tauri v2 + Rust)                          |
+--------------------------------+--------------------------------+
| Frontend (React 19 + Vite)     | Backend Core (Rust + Tokio)    |
| - Virtualized Staging Table    | - Embedded WebView Auth Trap   |
| - Disambiguation Drawer        | - 4-Stage Matching Pipeline    |
| - Audio Preview Singleton      | - Chunked Batch Mutation Pool  |
| - Real-Time Telemetry Console  | - SQLite Snapshot & Rollback   |
| - Transfer History Dashboard   | - OS Keyring & AES-256 Vault   |
+--------------------------------+--------------------------------+
```

---

## Getting Started

### Prerequisites

Ensure you have the following installed on your system:

- **Node.js**: v18.0.0 or higher
- **npm**: v9.0.0 or higher
- **Rust**: 1.80.0 or higher (`rustup default stable`)
- **Platform Dependencies**:
  - **Windows**: Microsoft Edge WebView2 (pre-installed on Windows 10/11) and C++ Build Tools.
  - **macOS**: Xcode Command Line Tools.
  - **Linux**: `libwebkit2gtk-4.1-dev`, `build-essential`, `curl`, `wget`, `file`, `libssl-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`.

### Installation

1. Clone the repository:
   ```bash
   git clone https://github.com/username/SoundShift.git
   cd SoundShift
   ```

2. Install frontend dependencies:
   ```bash
   npm install
   ```

3. Run in development mode:
   ```bash
   npm run tauri dev
   ```

4. Build a production installer:
   ```bash
   npm run tauri build
   ```

---

## Testing & Quality Assurance

SoundShift includes automated unit, integration, and benchmark test suites across both frontend and backend.

### Run Frontend Tests
```bash
npm test
```

### Run Rust Tests
```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

### Run Headless Matching Benchmark
```bash
cargo test --manifest-path src-tauri/Cargo.toml --test matching_benchmark -- --nocapture
```

### Run Code Linter & Typecheck
```bash
npm run build
cargo clippy --manifest-path src-tauri/Cargo.toml
```

---

## Security & Privacy

- **No Remote Telemetry:** SoundShift does not send usage metrics, error logs, or analytics to any remote server.
- **Local Credential Storage:** Session tokens and authentication states are saved in the operating system's native secret storage using standard platform APIs.
- **Direct Peer Networking:** All API communications are executed directly from your local network to Spotify and YouTube Music endpoints.

---

## Contributing

Contributions are welcome. Please read [CONTRIBUTING.md](.github/CONTRIBUTING.md) for details on code formatting, pull request workflows, and architectural guidelines.

---

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.
