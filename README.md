<div align="center">
  <img src="public/app-logo.png" alt="SoundShift logo" width="112" height="112" />
  <h1>SoundShift</h1>
  <p><strong>Local-first playlist migration with inspectable matching and safe rollback</strong></p>
  <p>
    <a href="https://github.com/sizwinz/SoundShift">Repository</a> ·
    <a href="https://github.com/sizwinz/SoundShift/issues">Issues</a> ·
    <a href="https://github.com/sizwinz/SoundShift/discussions">Discussions</a>
  </p>
</div>

SoundShift is an open-source cross-platform desktop app for moving playlists between Spotify and YouTube Music without uploading your library to a cloud service. It uses native WebView sessions for authentication, deterministic matching safeguards, a reviewable staging diff, durable transfer jobs, and local snapshots for recovery.

> SoundShift transfers playlist metadata only. It does not download, rip, encode, or store audio files.

## Why SoundShift

- **Local-first by design:** Credentials, snapshots, transfer history, and match data stay on your device. There is no SoundShift telemetry or hosted migration backend.
- **No manual DevTools setup:** Connect providers through isolated native login sessions instead of copying cookies or API headers by hand.
- **Review before mutation:** Every proposed destination match appears in a staging diff before tracks are written.
- **Conservative matching:** ISRC and normalized metadata checks are combined with duration-aware fuzzy matching. Uncertain candidates are routed to review instead of being silently accepted.
- **Durable transfers:** Jobs, operations, events, snapshots, and recovery state are persisted locally so interrupted work can be inspected and recovered.
- **Safe rollback:** Pre-mutation snapshots support restoring transfer changes from the local history view.
- **Large playlist UX:** The staging table uses virtualization for responsive review of large playlists.

## Supported providers

| Provider | Playlist read | Playlist write | Authentication |
| --- | ---: | ---: | --- |
| Spotify | Yes | Yes | Native WebView session |
| YouTube Music | Yes | Yes | Native WebView session |

Provider behavior can change independently of SoundShift. The app uses retries, throttling, reconciliation, and provider-specific mutation safeguards to verify what was actually written.

## Workflow

1. **Connect** Spotify and/or YouTube Music in the Accounts view.
2. **Select** a source playlist and destination provider.
3. **Analyze** tracks through the local matching pipeline.
4. **Review** exact, ambiguous, unresolved, and rejected candidates in the staging diff.
5. **Transfer** only the tracks you approve.
6. **Verify or recover** from Transfer History using durable events and snapshots.

## Matching safety

SoundShift favors transparent decisions over aggressive guesses:

- Local match cache and exact identifiers are checked first.
- Metadata is normalized before provider searches.
- Candidate confidence is anchored by track duration differences.
- Live versions, extended mixes, interviews, and other high-delta variants are rejected or sent for review.
- Ambiguous rows are not included in bulk transfer unless they satisfy the app's safety rules.

## Technology

- **Desktop:** Tauri v2
- **Core:** Rust 2021, Tokio, Reqwest, SQLite via rusqlite
- **Frontend:** React 19, TypeScript, Vite, Tailwind CSS
- **UX:** TanStack Virtual, Lucide icons, responsive dark-first interface
- **Credential storage:** OS keyring with encrypted local fallback where required

## Development setup

### Prerequisites

- Node.js 18 or newer
- npm 9 or newer
- Rust stable with `rustup`
- Platform dependencies required by Tauri v2
  - Windows: WebView2 and C++ Build Tools
  - macOS: Xcode Command Line Tools
  - Linux: WebKitGTK and GTK development packages

### Run locally

```bash
git clone https://github.com/sizwinz/SoundShift.git
cd SoundShift
npm install
npm run tauri dev
```

Build the desktop application with:

```bash
npm run tauri build
```

### Validate changes

```bash
npm test -- --run
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
```

The matching benchmark can be run with:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test matching_benchmark -- --nocapture
```

## Privacy and security

- No SoundShift analytics, telemetry, or third-party tracking.
- Authentication material is kept in the OS credential manager where available.
- Local SQLite data contains transfer state, caches, and snapshots; it is not uploaded by SoundShift.
- Network requests go directly to the connected streaming providers.
- Disconnecting a provider removes its stored credentials while preserving non-sensitive match history where applicable.

## Project status

SoundShift is under active development. Spotify and YouTube Music playlist migration are the current focus. APIs and provider internals may change as the upstream services evolve, so please report reproducible failures with provider, platform, playlist size, and relevant transfer-log details while omitting credentials or cookies.

## Contributing

Bug reports, provider compatibility reports, matching improvements, UI work, and documentation contributions are welcome. Please open an issue before large architectural changes and keep credentials, cookies, access tokens, and private playlist data out of issues and pull requests.

## License

SoundShift is licensed under the MIT License. See [LICENSE](LICENSE).
