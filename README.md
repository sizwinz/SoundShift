<div align="center">
  <img src="public/app-logo.png" alt="SoundShift Logo" width="96" height="96" />
  <h1>SoundShift</h1>
  <p><strong>Effortless, local-first playlist migration with live visual diffs and 1-click rollback.</strong></p>

  <p>
    <a href="https://github.com/sizwinz/SoundShift"><img src="https://img.shields.io/badge/Tauri-v2.x-blue?style=flat-square&logo=tauri" alt="Tauri v2" /></a>
    <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-stable-orange?style=flat-square&logo=rust" alt="Rust" /></a>
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

<div align="center">
<video src="https://github.com/user-attachments/assets/9983d9d2-e760-4070-9f3c-5011bd2af2d5" width="880" autoplay="autoplay"></video>
</div>

## What is SoundShift?

SoundShift is a fast, local desktop app that moves your music playlists between **Spotify** and **YouTube Music**.

Most playlist transfer tools require paid monthly subscriptions, ask for third-party cloud access, or demand that you manually copy API tokens from browser developer tools. SoundShift runs **100% locally on your computer**, connects through native login windows, lets you preview matches before writes, and records transfer history for explicit recovery and guarded rollback.

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

### Scheduled Two-Way Playlist Sync
Open **Sync**, link existing editable Spotify and YouTube Music playlists, and review their initial merged songs and order. Apply the reviewed merge before enabling the optional hourly schedule. Checks continue while the app is open, including when minimized; closing SoundShift stops scheduling.

Later runs propagate additions, removals, and ordering against a verified shared baseline. Intentional repeated songs remain separate occurrences. Approved mappings and exact matches can run automatically; conflicting edits, missing matches, incomplete reads, credential changes, and uncertain writes stop the pair for review. Names, descriptions, artwork, and visibility stay local.

Sync requires recognized provider capabilities and verifies the ordered result on both services. Spotify duplicate-occurrence removal remains unsupported; changes to a Spotify baseline containing duplicates also require attribution review. YouTube Music writes require occurrence identifiers and recognized responses. Inspection, reconciliation, pause/resume/cancel, and guarded recovery are available from the Sync page; uncertain mutations are never replayed automatically. Unlinking preserves both remote playlists and recorded run history.

### 1-Click Snapshot Rollback
Every transfer records an immutable snapshot and a durable mutation journal before writes. Transfer History lets you inspect outcomes, reconcile uncertain work, resume proven unsent or rejected operations, and request rollback. Rollback requires attributable occurrences and verified remote post-state. Legacy records and unsupported provider capabilities stop for manual review; whole-playlist deletion is refused when independent removal verification is unavailable.

### Smart Deduplication
Merging into a playlist you already use? SoundShift checks existing songs in the destination by ID, ISRC, and title/artist, skipping tracks that are already there so you do not end up with duplicates.

### Concurrent Worker Engine
Powered by Tokio and Rust, matching runs concurrently while destination playlist writes run in order with one writer. Transfers use bounded retry budgets, provider retry guidance, interruptible waits, and live progress logs. Confirmed operations are never automatically replayed.

### 100% Private & Local-First
Zero cloud servers. Zero analytics or tracking beacons. Your tokens, snapshots, and playlist history stay entirely inside local SQLite storage on your machine.

---

## Supported Services

| Service | Read Playlists | Create New Playlist | Append to Existing | Deduplication | Auth Method |
| :--- | :---: | :---: | :---: | :---: | :--- |
| **Spotify** | Yes | Yes | Yes | Yes | Native WebView session |
| **YouTube Music** | Yes | Yes | Yes | Yes | Native WebView session |

*Playlist writes are serialized across transfers, sync, and recovery. The transfer workflow still refuses unsupported YouTube Music duplicate-preservation selections. Sync uses a separate occurrence-aware insertion path with receipt and ordered post-state verification. Live provider eligibility and native platform acceptance remain separate from local automated tests.*

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
[5. Batch Transfer]    -> Ordered playlist writes with deduplication and live telemetry
         |
[6. Snapshot Safety]   -> Durable outcomes, explicit recovery, and guarded rollback
```

---

## Quick Start

### Download
Pre-built installers for Windows, macOS, and Linux will be available on the [Releases](https://github.com/sizwinz/SoundShift/releases) page.

### Build from Source

#### Prerequisites
- **Node.js** (v22.12+; v24 recommended) and **npm**
- **Rust** (current stable toolchain)
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
