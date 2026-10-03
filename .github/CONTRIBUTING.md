# Contributing to SoundShift

Thank you for your interest in contributing to SoundShift. SoundShift is an open-source, local-first playlist migration tool. We welcome bug reports, feature suggestions, and code contributions.

---

## Code of Conduct & Standards

To maintain a production-grade codebase, all contributors must adhere to these engineering standards:

1. **Local-First & Zero Telemetry Invariant:**
   Never introduce remote telemetry, centralized tracking, or cloud proxies. SoundShift must remain strictly 100% local-first.
2. **Permanent Fixes Only:**
   Investigate root causes. Avoid patch fixes, temporary band-aids, or error suppression hacks.
3. **No Unneeded Dependencies:**
   Rely on standard libraries and existing dependencies whenever possible before adding new external crates or npm packages.
4. **Desktop & Mobile Parity:**
   Ensure layouts and responsive interactions remain robust and glitch-free.
5. **Clean Code & Typography:**
   - Do not use em dashes in documentation or commit messages (use hyphens, colons, or bullet points).
   - Write self-documenting code with comments reserved for complex domain invariants and non-obvious algorithms.

---

## Development Setup

1. Fork and clone the repository.
2. Ensure you have Node.js 18+ and Rust 1.80+ installed.
3. Install frontend dependencies:
   ```bash
   npm install
   ```
4. Start the development server:
   ```bash
   npm run tauri dev
   ```

---

## Verification Before Submitting

Before opening a pull request, ensure all tests and quality checks pass locally:

```bash
# 1. Run frontend unit tests
npm test

# 2. Run TypeScript build verification
npm run build

# 3. Run Rust unit tests
cargo test --manifest-path src-tauri/Cargo.toml

# 4. Run Rust Clippy linter
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
```

---

## Commit Guidelines

SoundShift follows Conventional Commits specification:

- `feat(scope): add new feature`
- `fix(scope): resolve bug`
- `docs: update documentation`
- `test(scope): add or improve tests`
- `refactor(scope): restructure code without changing behavior`
