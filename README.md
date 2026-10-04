<div align="center">
  <img src="public/app-logo.png" alt="SoundShift Logo" width="96" height="96" />
  <h1>SoundShift</h1>
  <p><strong>Effortless, local-first playlist migration with live visual diffs and 1-click rollback.</strong></p>

  <p>
    <a href="https://github.com/sizwinz/SoundShift"><img src="https://img.shields.io/badge/Tauri-v2.x-blue?style=flat-square&logo=tauri" alt="Tauri v2" /></a>
    <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.80+-orange?style=flat-square&logo=rust" alt="Rust" /></a>
    <a href="https://react.dev/"><img src="https://img.shields.io/badge/React-19-61dafb?style=flat-square&logo=react" alt="React 19" /></a>
    <a href="https://www.typescriptlang.org/"><img src="https://img.shields.io/badge/TypeScript-5.6-blue?style=flat-square&logo=typescript" alt="TypeScript" /></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-green?style=flat-square" alt="MIT License" /></a>
    <a href="PRIVACY.md"><img src="https://img.shields.io/badge/Telemetry-Zero-brightgreen?style=flat-square" alt="Zero Telemetry" /></a>
  </p>

  <p>
    <a href="#features">Features</a> &bull;
    <a href="#quick-start">Quick Start</a> &bull;
    <a href="#how-it-works">How It Works</a> &bull;
    <a href="#supported-services">Supported Services</a> &bull;
    <a href="CONTRIBUTING.md">Contributing</a> &bull;
    <a href="PRIVACY.md">Privacy Policy</a>
  </p>
</div>

---

## What is SoundShift?

SoundShift is a fast, local desktop app that moves your music playlists between **Spotify** and **YouTube Music**.

Most playlist transfer tools require paid monthly subscriptions, ask for third-party cloud access, or demand that you manually copy API tokens from browser developer tools. SoundShift takes a different approach: it runs **100% locally on your computer**, connects through native login windows, lets you preview every match before anything changes, and lets you undo any transfer with a single click.

> **Metadata only:** SoundShift transfers playlist track metadata (titles, artists, and album info). It never downloads, rips, or streams audio files.

---

## Features

### Direct In-App Login
No developer keys, Spotify client secrets, or copied browser headers required. Log in directly through isolated native WebView windows. Credentials stay safely in your operating system's keychain.

### Staging Diff & Match Inspector
Never wonder what got transferred. Before a single song is added to your destination playlist, SoundShift displays a full visual diff:
- **Exact Matches (Green)**: Matched with high confidence using ISRC codes, normalized titles, and duration checks.
- **Ambiguous Matches (Amber)**: Multiple candidate songs found; pick the right one manually in seconds.
- **Unresolved / Missing (Red)**: Clearly highlighted so you know exactly which tracks need attention.

### 1-Click Snapshot Rollback
Accidentally added tracks to the wrong playlist or changed your mind? Every transfer automatically records an immutable snapshot of your playlist before making changes. Restore your original playlist anytime from the Transfer History screen.

### Smart Deduplication
Merging into a playlist you already use? SoundShift checks existing songs in the destination by ID, ISRC, and title/artist, skipping tracks that are already there so you do not end up with duplicates.

### Concurrent Worker Engine
Powered by Tokio and Rust, transfers run with user-selectable worker concurrency (1 to 8 workers), adaptive rate limiting, exponential backoff, and live real-time progress logs.

### 100% Private & Local-First
Zero cloud servers. Zero analytics or tracking beacons. Your tokens, snapshots, and playlist history stay entirely inside local SQLite storage on your machine.

---

## Supported Services

| Service | Read Playlists | Create New Playlist | Append to Existing | Deduplication | Auth Method |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **Spotify** | Yes | Yes | Yes | Yes | Native WebView session |
| **YouTube Music** | Yes | Yes | Yes | Yes | Native WebView session |

*Note: YouTube Music playlist mutations enforce single-writer consistency to avoid server conflict collisions.*

---

## How It Works

```text
[1. Connect Accounts]  -> Native WebView authentication (stored in OS Keychain)
         |
[2. Ingest Playlist]   -> Cached locally in SQLite for instant browsing
         |
[3. Match Pipeline]    -> Multi-pass matching: ISRC -> Normalization -> Duration check
         |
[4. Visual Staging]    -> Review exact and ambiguous matches before transferring
         |
[5. Batch Transfer]    -> Concurrent worker tasks with deduplication and live telemetry
         |
[6. Snapshot Safety]   -> Pre-mutation state saved in SQLite with 1-click rollback
```

---

## Quick Start

### Download
Pre-built installers for Windows, macOS, and Linux will be available on the [Releases](https://github.com/sizwinz/SoundShift/releases) page.

### Build from Source

#### Prerequisites
- **Node.js** (v18+) and **npm**
- **Rust** (stable toolchain, 1.80+)
- Platform build dependencies (MSVC on Windows, Xcode tools on macOS, WebKitGTK on Linux)

#### Steps

1. Clone the repository:
   ```bash
   git clone https://github.com/sizwinz/SoundShift.git
   cd SoundShift
   ```

2. Install dependencies:
   ```bash
   npm install
   ```

3. Run in development mode:
   ```bash
   npm run tauri dev
   ```

4. Build a release installer:
   ```bash
   npm run tauri build
   ```

---

## Testing & Quality

SoundShift maintains strict quality gates across both the Rust systems core and React frontend:

```bash
# Run backend test suite
cargo test --manifest-path src-tauri/Cargo.toml

# Run backend linter with zero warnings
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings

# Run frontend tests
npm test

# Run frontend build
npm run build
```

---

## Documentation

- [Contributing Guidelines](CONTRIBUTING.md): Instructions for setup, coding standards, and submitting pull requests.
- [Privacy & Security Policy](PRIVACY.md): Detailed information on local storage, credential management, and networking.
- [Code of Conduct](CODE_OF_CONDUCT.md): Community pledge and standards for contributors.

---

## License

SoundShift is open source software licensed under the [MIT License](LICENSE).
