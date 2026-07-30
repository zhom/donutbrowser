# Contributing to Donut Browser

Contributions are welcome. Don't open a PR just to get added to the contributors list. Reviewing PRs takes time, so open one only if you believe the change improves Donut for yourself and others. For a significant change, get in touch with the maintainer first.

## AI Policy

AI can write your code. It cannot speak for you, and it cannot be a co-author.

- Disclose it, always. Every PR must say whether AI was used. The template has two boxes and exactly one must be ticked. Neither ticked, both ticked, or the section deleted, and a bot closes the PR. Issues carry the same question.
- No AI co-authors. A commit carrying a `Co-Authored-By:` trailer naming an AI tool, or a "Generated with ..." attribution, closes the PR. Strip them before pushing; `git commit --amend` or a rebase is enough. Most coding agents add these by default, so check.
- Write your own words. Commit messages, PR descriptions, and replies in review must be yours. Broken English is welcome here; people contribute from everywhere and I would much rather read theirs. AI English is not welcome: it is long, evenly confident, and costs a reviewer time in proportion to how good it sounds.

Disclosing AI use is never held against you. Hiding it is what gets a PR closed.

## Before Starting

- Search existing PRs related to that issue
- Confirm no other contributors are working on the same issue
- Check if the feature aligns with the project's goals

## Contributor License Agreement

By contributing, you agree your contributions will be licensed under the same terms as the project. See [Contributor License Agreement](CONTRIBUTOR_LICENSE_AGREEMENT.md). This lets contributions be used in the open source version (AGPL-3.0) and commercially licensed. You retain all rights to use your contributions elsewhere.

## Development Setup

### Using Nix (recommended)

```bash
nix run .#setup     # Install dependencies
nix run .#tauri-dev  # Start development server
nix run .#test       # Run all checks
```

Or enter the dev shell: `nix develop`

### Manual Setup

Requirements:

- Node.js (see `.node-version`)
- pnpm
- Rust + Cargo (latest stable)
- [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/)

```bash
git checkout -b feature/my-feature-name
pnpm install
pnpm tauri dev
```

## Quality Checks

Run before every commit:

```bash
pnpm format && pnpm lint && pnpm test
```

This runs:

- Biome: JS/TS linting and formatting
- Clippy + rustfmt: Rust linting and formatting
- typos: Spellcheck (allowlist in `_typos.toml`)
- CodeQL: Security analysis (JS, Actions, Rust), runs in CI
- Unit tests: 330+ Rust tests
- Integration tests: proxy, sync e2e

### Running CodeQL locally

```bash
# Install: brew install codeql
codeql pack download codeql/javascript-queries codeql/rust-queries

# JavaScript
codeql database create /tmp/codeql-js --language=javascript --source-root=.
codeql database analyze /tmp/codeql-js --format=sarifv2.1.0 --output=/tmp/js.sarif codeql/javascript-queries

# Rust
codeql database create /tmp/codeql-rust --language=rust --source-root=.
codeql database analyze /tmp/codeql-rust --format=sarifv2.1.0 --output=/tmp/rust.sarif codeql/rust-queries
```

## Key Rules

- Translations: Any UI text change must be reflected in all 9 locale files (`src/i18n/locales/`)
- Tauri commands: If you modify Tauri commands, the `test_no_unused_tauri_commands` test will catch unused ones
- No hardcoded colors: Use theme CSS variables (see `src/lib/themes.ts`), never Tailwind color classes like `text-red-500`
- No lock file changes: Don't update `pnpm-lock.yaml` or `Cargo.lock` unless updating dependencies is the purpose of the PR
- AGPL-3.0: This project is AGPL-licensed. Derivatives must be open source with the same license

## Pull Request Guidelines

- Fill the PR description template
- Reference related issues (`Fixes #123` or `Refs #123`)
- Include screenshots/videos for UI changes
- Ensure "Allow edits from maintainers" is checked

## Architecture

- Frontend: Next.js (React), `src/`
- Backend: Tauri (Rust), `src-tauri/src/`
- Proxy Worker: Detached process for proxy tunneling, `src-tauri/src/bin/proxy_server.rs`
- Sync: Cloud sync via S3-compatible storage, `src-tauri/src/sync/`, `donut-sync/`
- Browsers: Wayfern (Chromium-based anti-detect)

## Getting Help

- Issues: Bug reports and feature requests
- Discussions: Questions and general discussion
