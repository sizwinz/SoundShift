<!-- GSD:project-start source:PROJECT.md -->

## Project

**SoundShift**

SoundShift is an open-source, local-first cross-platform desktop application engineered to migrate, sync, and back up music playlists across streaming services (launching with Spotify and YouTube Music). It requires zero API keys or manual DevTools header extraction from the user, utilizing embedded native WebView sessions, deterministic multi-stage matching heuristics, an inspectable staging "diff" interface, and 1-click snapshot rollback.

**Core Value:** Deterministic, transparent playlist migrations executed entirely client-side with zero subscription barriers, zero user-supplied API keys, and zero library data loss.

### Constraints

- **Runtime & Footprint**: Tauri v2 + Rust Core with React 19 + TypeScript + Vite frontend. Sub-15MB installer footprint, <= 85MB idle RAM, <= 190MB during 5,000-track diff calculations.
- **Matching Accuracy**: Target >= 98% automated match accuracy on standard studio recordings.
- **List Performance**: Must sustain smooth 60 FPS scrolling on 15,000+ track entities via `@tanstack/react-virtual`.
- **Privacy & Security**: Zero external telemetry. All credentials and snapshots remain strictly on the local machine.
- **Networking**: Requests executed via `reqwest` (HTTP/2) with header spoofing, exponential backoff, and user-configurable concurrency limits (1 to 8 workers).

<!-- GSD:project-end -->

<!-- GSD:stack-start source:research/STACK.md -->

## Technology Stack

## Recommended Stack

### Core Technologies

| Technology | Version | Purpose | Why Recommended |
|------------|---------|---------|-----------------|
| Tauri | v2.x | Desktop Shell & Native OS Runtime | Sub-15MB installer, minimal RAM footprint (<85MB idle), direct native WebView control, and secure IPC boundaries without Electron overhead. |
| Rust | 1.80+ (2021 Edition) | Systems Core & Protocol Execution | Memory safety, high-throughput asynchronous execution, native HTTP/2 handling, and tight OS keychain integration. |
| React | 19.x | Presentation & Reactive UI | Modern concurrent rendering, clean declarative component lifecycle, and broad ecosystem compatibility. |
| TypeScript | 5.5+ | Type Safety | Enforces strict compile-time types across IPC payloads, driver interfaces, and database entities. |
| Vite | 6.x | Frontend Tooling & Dev Server | Instant HMR, optimized tree-shaking, and seamless integration with Tauri v2 build pipeline. |
| SQLite (via `rusqlite`) | 0.32+ | Embedded Local Storage | Zero-latency local caching of ISRC track matches, session tokens, transfer history, and snapshot rollbacks. Bundled with SQLite 3.45+. |

### Supporting Libraries

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `@tanstack/react-virtual` | v3.x | DOM Virtualization | Essential for rendering 10,000+ tracks smoothly at 60 FPS without unbounded memory consumption. |
| Tailwind CSS | 4.x / 3.4+ | Utility-first Styling | Enables true-black AMOLED (`#000000`, `#09090b`) dark-first UI design system and rapid styling. |
| `shadcn/ui` + Radix Primitives | Latest | Accessible UI Components | Production-grade accessible dialogs, drawers (disambiguation), tooltips, and badges. |
| `tokio` | 1.40+ | Asynchronous Runtime in Rust | Drives multi-threaded worker pools, exponential backoff timers, and concurrent network requests. |
| `reqwest` | 0.12+ | HTTP Client with HTTP/2 & TLS | Connection pooling, custom header management (InnerTube authorization, SAPISIDHASH), and request throttling. |
| `keyring` (Rust crate) | 3.x | Secure Credential Storage | Interacts with native OS secret stores (Windows Credential Locker, macOS Keychain, Linux Secret Service). |
| `strsim` or `fuzzy-matcher` | 0.11+ | Fuzzy String Matching | Levenshtein and Jaro-Winkler metric calculation for Pass 3 duration-anchored matching. |
| Lucide React | Latest | Clean System Icons | High-contrast, minimal iconography for controls and status badges. |

### Development Tools

| Tool | Purpose | Notes |
|------|---------|-------|
| `@tauri-apps/cli` | v2.x | Build and dev orchestrator | Manages native bundling and frontend proxying. |
| `cargo-audit` / `cargo-clippy` | Latest | Security and lint verification | Enforces strict Rust idioms and dependency security. |

## Installation

# Frontend Core & UI

# Radix UI Primitives (shadcn/ui backing)

# Dev Dependencies

# Rust Cargo.toml dependencies

## Alternatives Considered

| Recommended | Alternative | When to Use Alternative |
|-------------|-------------|-------------------------|
| Tauri v2 | Electron 33+ | If requiring complex Chrome DevTools extensions or Chromium-specific native APIs not covered by OS WebViews. (Rejected due to 150MB+ bundle size and high memory usage). |
| `rusqlite` (embedded) | `sqlx` (async) | If connection pooling over remote PostgreSQL/MySQL was required. For embedded desktop SQLite, `rusqlite` with synchronous local access in Tokio blocking threads is simpler, faster, and avoids async connection lock overhead. |
| `@tanstack/react-virtual` | `react-window` / `react-virtualized` | Legacy packages. TanStack Virtual is headless, lightweight, and specifically optimized for modern React 19 concurrent features. |
| OS `keyring-rs` + AES SQLite | Plaintext SQLite storage | Never for authentication tokens. Security and privacy dictate OS keychain or AES-256 encrypted session storage. |

## What NOT to Use

| Avoid | Why | Use Instead |
|-------|-----|-------------|
| Electron | Heavy binary footprint (>150MB installer, >250MB baseline RAM), slow cold launch. | Tauri v2 (<15MB installer, <85MB RAM). |
| `yt-dlp` / local audio encoders | Violates SoundShift non-goals; causes copyright liabilities and huge disk/network strain. | Strict metadata transfer only. |
| Pure client-side browser fetch | Browser CORS policies block YouTube Music and Spotify internal endpoints. | Rust backend via `reqwest` bypassing CORS with custom headers. |
| Web scraping via raw regex on HTML | Fragile and immediately breaks on DOM class name mutations. | InnerTube JSON endpoints and structured API endpoints. |

## Stack Patterns by Variant

- Tauri utilizes Microsoft Edge WebView2 (Evergreen runtime installed on Windows 10/11).
- Session credentials store in Windows Credential Manager via `keyring-rs`.
- Tauri utilizes WKWebView.
- Session credentials store in macOS Keychain via `keyring-rs`.
- Tauri utilizes WebKitGTK.
- Session credentials store in Freedesktop Secret Service / KWallet via `keyring-rs`, with fallback to encrypted SQLite (AES-256-GCM) if headless secret service is unavailable.

## Version Compatibility

| Package A | Compatible With | Notes |
|-----------|-----------------|-------|
| Tauri v2.0 | Rust 1.77.2+ | Requires Rust 2021 edition and modern C++ build tools for native components. |
| React 19 | `@tanstack/react-virtual` v3.x | Fully compatible with React 19 forwardRef and concurrent transitions. |
| `rusqlite` 0.32 | `bundled` feature | Avoids OS-specific SQLite version mismatches across user machines. |

## Sources

- Tauri Documentation (`/tauri-apps/tauri-docs`) - Tauri v2 architecture, multi-window and webview guidelines.
- TanStack Virtual (`/tanstack/virtual`) - Virtualized list rendering benchmarks and React 19 compatibility.
- Official Cargo docs - crate version validation for rusqlite, tokio, reqwest, and keyring.

<!-- GSD:stack-end -->

<!-- GSD:conventions-start source:CONVENTIONS.md -->

## Conventions

Conventions not yet established. Will populate as patterns emerge during development.
<!-- GSD:conventions-end -->

<!-- GSD:architecture-start source:ARCHITECTURE.md -->

## Architecture

Architecture not yet mapped. Follow existing patterns found in the codebase.
<!-- GSD:architecture-end -->

<!-- GSD:skills-start source:skills/ -->

## Project Skills

No project skills found. Add skills to any of: `.agents/skills/`, `.agents/skills/`, `.cursor/skills/`, `.github/skills/`, or `.codex/skills/` with a `SKILL.md` index file.
<!-- GSD:skills-end -->

<!-- GSD:workflow-start source:GSD defaults -->

## GSD Workflow Enforcement

Before using Edit, Write, or other file-changing tools, start work through a GSD command so planning artifacts and execution context stay in sync.

Use these entry points:
- `/gsd-quick` for small fixes, doc updates, and ad-hoc tasks
- `/gsd-debug` for investigation and bug fixing
- `/gsd-execute-phase` for planned phase work

Do not make direct repo edits outside a GSD workflow unless the user explicitly asks to bypass it.
<!-- GSD:workflow-end -->

<!-- GSD:profile-start -->

## Developer Profile

> Profile not yet configured. Run `/gsd-profile-user` to generate your developer profile.
> This section is managed by `generate-claude-profile` -- do not edit manually.
<!-- GSD:profile-end -->
