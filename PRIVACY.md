# Privacy & Security Policy

SoundShift is engineered from the ground up as a strictly local-first desktop application. Your music library, credentials, and listening habits are your personal data. SoundShift does not collect, transmit, monetize, or inspect them.

---

## Core Privacy Principles

### 1. Zero External Telemetry
- SoundShift contains no analytics trackers, crash reporters, telemetry beacons, or third-party tracking scripts.
- No user identity, IP address, device fingerprint, or usage pattern is ever collected or sent to a SoundShift server.
- The application does not ping any central SoundShift server on startup, during transfers, or upon errors.

### 2. Direct-to-Provider Networking
- All network traffic originating from SoundShift connects directly to the official streaming provider endpoints (Spotify, YouTube Music).
- There is no intermediary proxy server, cloud relay, or hosted migration worker.
- Transfer mutations, search queries, and playlist metadata reads happen entirely between your local computer and the respective music service.

### 3. Local Credential Storage
- Session credentials acquired through native WebView logins are stored securely in your operating system's native secret storage:
  - **Windows**: Windows Credential Manager via the OS Data Protection API.
  - **macOS**: Keychain Services.
  - **Linux**: Freedesktop Secret Service / KWallet.
- If native OS keyring access is unavailable, credentials fall back to an AES-256-GCM encrypted envelope bound to a host-derived machine key.
- Tokens and session cookies are never written to plaintext log files.

### 4. Local SQLite Persistence
- SoundShift stores migration jobs, track matching caches, pre-mutation snapshots, and transfer history in a local embedded SQLite database located in your standard user application directory.
- This database never syncs to a remote cloud service.
- You can inspect, back up, or wipe this database at any time by deleting the local application directory.

### 5. Instant Provider Disconnection
- When you click "Disconnect" in the Accounts view, SoundShift immediately purges the corresponding access tokens and session cookies from your OS credential vault and local database.
- Non-sensitive match caches remain available locally to speed up future transfers, or can be cleared at will.

---

## Security Invariants

- **Content Security Policy (CSP)**: Tauri v2 enforces a strict Content Security Policy restricting execution to local application bundles and connections solely to authorized streaming service domains.
- **IPC Isolation**: Global Tauri IPC exposure (`withGlobalTauri`) is disabled to prevent arbitrary JavaScript contexts from accessing native backend commands.
- **Metadata-Only Operations**: SoundShift only reads and writes playlist metadata (titles, artists, albums, ISRCs). It never attempts to download, decode, or distribute audio streams.

---

## Reporting Vulnerabilities

If you discover a potential security vulnerability in SoundShift, please report it privately:

- Open a private security advisory on GitHub or email the repository maintainers directly.
- Please do not publish security issues in public GitHub issues until a remediation patch has been released.
