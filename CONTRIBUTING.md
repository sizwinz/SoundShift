# Contributing to SoundShift

Thank you for your interest in contributing to SoundShift. SoundShift is an open-source, local-first desktop application built with Tauri v2, Rust, React 19, TypeScript, and Tailwind CSS.

---

## Code of Conduct

All contributors and maintainers are expected to adhere to our [Code of Conduct](CODE_OF_CONDUCT.md). Please ensure respectful and constructive collaboration at all times.

---

## How Can You Contribute?

- **Provider Drivers**: Add support for new streaming platforms (e.g., Apple Music, Tidal, Deezer) or improve existing Spotify / YouTube Music adapters.
- **Matching Heuristics**: Optimize track matching accuracy, fuzzy string scoring, and catalog normalization.
- **UI / UX Polish**: Enhance virtualized list performance, dark-mode themes, keyboard navigation, and responsive controls.
- **Documentation & Testing**: Add integration tests, improve developer docs, or expand unit test coverage.

---

## Local Development Setup

### Prerequisites

- **Node.js**: Version 18 or newer
- **Rust Toolchain**: Stable edition (2021 Edition, 1.80+) via `rustup`
- **Platform Native Dependencies**:
  - **Windows**: Microsoft Edge WebView2 Evergreen runtime and C++ Build Tools (MSVC)
  - **macOS**: Xcode Command Line Tools
  - **Linux**: `webkit2gtk-4.1`, `gtk3`, `libsoup-3.0`, and `libappindicator3` development packages

### Getting Started

1. Fork the repository and clone your fork locally:
   ```bash
   git clone https://github.com/your-username/SoundShift.git
   cd SoundShift
   ```

2. Install JavaScript dependencies:
   ```bash
   npm install
   ```

3. Launch the desktop development environment with Hot Module Replacement (HMR):
   ```bash
   npm run tauri dev
   ```

---

## Quality & Verification Standards

Before submitting a pull request, ensure all tests, lint checks, and builds pass cleanly:

### 1. Rust Code Quality & Tests
```bash
# Run unit and integration tests
cargo test --manifest-path src-tauri/Cargo.toml

# Run clippy with strict zero-warning enforcement
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
```

### 2. Frontend Tests & Production Build
```bash
# Run Vitest suite
npm test

# Run TypeScript compilation and Vite production bundle
npm run build
```

---

## Engineering Guidelines

- **No Em Dashes**: Never use em dashes ("—"). Use hyphens ("-"), colons (":"), parentheses, or structured bullet points instead.
- **Minimal / No Emojis**: Do not use decorative emojis in code, commit messages, or technical documentation. Use clean icons, badges, or structured text.
- **Permanent Fixes**: Address underlying root causes. Avoid patch fixes, temporary band-aids, or error suppression hacks.
- **Privacy First**: Never add external telemetry, analytics, or remote logging. SoundShift must remain strictly local-first.
- **Sanitized Reports & PRs**: Never include personal authentication tokens, session cookies, private API payloads, or confidential credentials in issues, tests, or pull requests.

---

## Pull Request Workflow

1. Create a feature branch from `master`:
   ```bash
   git checkout -b feat/your-feature-name
   ```
2. Commit your changes using conventional commit messages (e.g., `feat(engine): ...`, `fix(ui): ...`).
3. Ensure all local tests and lints pass.
4. Push your branch to GitHub and open a Pull Request against `master`.
5. Provide a clear description of the problem solved, changes made, and manual testing steps.
