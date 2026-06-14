# ⛔ ABSOLUTE GIT RULE — READ FIRST (2026-06-11)

**NEVER run any git command that modifies git history OR the working tree, in ANY repo** (wayfern, wayfern-macos, wayfern-test, donutbrowser, build/src), **unless the user EXPLICITLY authorizes that exact command.** Forbidden without per-command authorization: `commit`, `revert`, `cherry-pick`, `restore`, `checkout` (files/branches), `reset`, `rebase`, `merge`, `stash`, `clean`, `apply`, `add`, `rm`, `push`, any force op. Only read-only git (`status`, `log`, `show`, `diff`, `ls-files`, `rev-parse`) is allowed without asking. **Authorization is per-command: 1 explicit authorization = exactly 1 command.** If a git mutation seems needed, STOP and ask for that one command.

---

# Project Guidelines

> **NOTE**: CLAUDE.md is a symlink to AGENTS.md — editing either file updates both.
> After significant changes (new modules, renamed files, new directories), re-evaluate the Repository Structure below and update it if needed.

## Repository Structure

```
donutbrowser/
├── src/                              # Next.js frontend
│   ├── app/                          # App router (page.tsx, layout.tsx)
│   ├── components/                   # 50+ React components (dialogs, tables, UI)
│   ├── hooks/                        # Event-driven React hooks
│   ├── i18n/locales/                 # Translations (en, es, fr, ja, ko, pt, ru, vi, zh)
│   ├── lib/                          # Utilities (themes, toast, browser-utils)
│   └── types.ts                      # Shared TypeScript interfaces
├── src-tauri/                        # Rust backend (Tauri)
│   ├── src/
│   │   ├── lib.rs                    # Tauri command registration (100+ commands)
│   │   ├── browser_runner.rs         # Profile launch/kill orchestration
│   │   ├── browser.rs               # Browser trait & launch logic
│   │   ├── profile/                  # Profile CRUD (manager.rs, types.rs)
│   │   ├── proxy_manager.rs         # Proxy lifecycle & connection testing
│   │   ├── proxy_server.rs          # Local proxy binary (donut-proxy)
│   │   ├── proxy_storage.rs         # Proxy config persistence (JSON files)
│   │   ├── api_server.rs            # REST API (utoipa + axum)
│   │   ├── mcp_server.rs            # MCP protocol server
│   │   ├── sync/                    # Cloud sync (engine, encryption, manifest, scheduler)
│   │   ├── vpn/                     # WireGuard tunnels
│   │   ├── wayfern_manager.rs       # Wayfern (Chromium) browser management
│   │   ├── downloader.rs           # Browser binary downloader
│   │   ├── extraction.rs           # Archive extraction (zip, tar, dmg, msi)
│   │   ├── settings_manager.rs     # App settings persistence
│   │   ├── cookie_manager.rs       # Cookie import/export
│   │   ├── extension_manager.rs    # Browser extension management
│   │   ├── group_manager.rs        # Profile group management
│   │   ├── synchronizer.rs         # Real-time profile synchronizer
│   │   ├── daemon/                 # Background daemon + tray icon (currently disabled)
│   │   └── cloud_auth.rs           # Cloud authentication
│   ├── tests/                      # Integration tests
│   └── Cargo.toml                  # Rust dependencies
├── donut-sync/                     # NestJS sync server (self-hostable)
│   └── src/                        # Controllers, services, auth, S3 sync
├── docs/                           # Documentation (self-hosting guide)
├── flake.nix                       # Nix development environment
└── .github/workflows/              # CI/CD pipelines
```

## Testing and Quality

- After making changes, run `pnpm format && pnpm lint && pnpm test` at the root of the project
- Always run this command before finishing a task to ensure the application isn't broken
- `pnpm lint` includes spellcheck via [typos](https://github.com/crate-ci/typos). False positives can be allowlisted in `_typos.toml`
- The full `pnpm test` output dumps every test name (≈400+ lines) which burns context for no signal. Filter:
  `pnpm test 2>&1 | grep -E "test result|panicked|FAILED"` — four "test result: ok" lines means everything passed.

## Logs (when debugging a running app)

Three log surfaces, in order of usefulness:

- **Donut Browser GUI** — `~/Library/Logs/com.donutbrowser/DonutBrowser.log` on macOS (newest = active session; older `DonutBrowser_<date>.log` are rotated). The GUI / Tauri / `browser_runner` / `proxy_manager` / `sync` all log here. Search for `Wayfern`, `Starting local proxy`, `Configured local proxy` to find a launch chain. Dev builds write to `DonutBrowserDev.log` instead.
- **donut-proxy worker** — `$TMPDIR/donut-proxy-<config_id>.log`. One file per proxy worker process (each profile launch spawns a fresh one). Map a worker to its launch via the `Cleanup: browser PID X is dead, stopping proxy worker <id>` lines in DonutBrowser.log, or by mtime. CONNECT requests, upstream accept/reject (status lines like `HTTP/1.1 402 user reached limit`), and tunnel errors are at INFO/WARN — anything finer is at TRACE and requires `RUST_LOG=donut_proxy=trace`. The `Upstream CONNECT response coalesced N byte(s) of payload — these would be dropped without forwarding` warning marks a real bug in `handle_connect_from_buffer` if it ever fires.

Linux/Windows swap `~/Library/Logs/com.donutbrowser/` for the platform-appropriate location (see `app_dirs::app_name()`), but the `$TMPDIR` worker logs are always under the system temp dir.

## Code Quality

- Don't leave comments that don't add value
- Don't duplicate code unless there's a very good reason; keep the same logic in one place
- Anytime you make changes that affect copy or add new text, it has to be reflected in all translation files

## Translations (mandatory)

- Never write user-facing strings as raw English literals in JSX, toast messages, dialog titles/descriptions, button labels, placeholders, table headers, tooltips, or empty-state text. Always go through `t("namespace.key")` from `useTranslation()`.
- This applies to every component under `src/` — including new ones. If a component doesn't already import `useTranslation`, add it.
- Adding a new string means adding the key to ALL nine locale files in `src/i18n/locales/` (en, es, fr, ja, ko, pt, ru, vi, zh) — not just `en.json`. The English version alone is incomplete work.
- Reuse existing keys (`common.buttons.*`, `common.labels.*`, `createProfile.*`, etc.) before creating new namespaces. Check `en.json` first.
- Strings excluded from this rule: `console.log/warn/error`, dev-only debug labels, internal IDs, CSS class names, type names. If unsure whether a string renders to the user, assume it does and translate it.
- **Never use `t(key, "fallback")` with a default-value second argument.** The 2-arg form is forbidden — every key must exist in every locale file before the call site lands. Fallbacks mask missing translations: a key missing from `ru.json` will silently render the English fallback to Russian users, so the bug never surfaces in CI or review. Only call `t("namespace.key")`. If a translation is missing for any locale, that's a bug to fix at the JSON, not a hole to paper over at the call site.
- Empty-string values in non-English locales are also forbidden — a locale either has the right translation or it has the same content as English; never `""`. If a particular language doesn't need a particular phrase (e.g. a suffix that doesn't grammatically apply), refactor the JSX to use a single interpolated key (`t("foo.bar", { name })` with `"...{{name}}..."` in each locale) instead of splitting prefix/suffix.
- When adding or removing keys across all nine locales, use a one-shot Python script in `/tmp/` that loads each `*.json`, mutates it, and writes it back. Nine sequential `Edit` calls drift (typos, ordering differences) and burn tokens; a single script keeps the locales in lockstep and is easy to throw away.

## Backend error codes (mandatory)

User-facing errors returned from a Tauri command MUST be JSON `{ "code": "FOO_BAR", "params": { … } }` strings — never raw English (`format!("Failed to …")`). The frontend resolves the code via `translateBackendError(t, err)` from `src/lib/backend-errors.ts`. Adding a new code requires four parallel edits:

1. Emit the JSON from Rust:
   ```rust
   return Err(serde_json::json!({ "code": "FOO_BAR" }).to_string());
   // or with params:
   return Err(serde_json::json!({ "code": "FOO_BAR", "params": { "n": "5" } }).to_string());
   ```
2. Add `"FOO_BAR"` to the `BackendErrorCode` union in `src/lib/backend-errors.ts`.
3. Add a `case "FOO_BAR":` in the switch that returns `t("backendErrors.fooBar", …)`.
4. Add `backendErrors.fooBar` to all nine locale files.

Raw error strings reach the user untranslated; that's the bug pattern this rule blocks.

## Sub-page Dialog mode

A `<Dialog>` becomes a first-class app sub-page (no modal overlay, no center positioning) when `subPage` is passed. Pages like Account, Settings, Proxy Management, and Extension Management use this. The pattern for a sub-page with tabs:

```tsx
<Dialog open={isOpen} onOpenChange={onClose} subPage={subPage}>
  <DialogContent className="max-w-2xl flex flex-col">
    <Tabs defaultValue="account">
      <TabsList
        className={cn(
          "w-full",
          subPage &&
            "!bg-transparent !p-0 !h-auto !rounded-none justify-start gap-4",
        )}
      >
        <TabsTrigger
          value="account"
          className={cn(
            "flex-1",
            subPage &&
              "!flex-none !rounded-none !bg-transparent !shadow-none data-[state=active]:!bg-transparent data-[state=active]:!text-foreground data-[state=active]:!shadow-none text-muted-foreground hover:text-foreground !px-1 !py-1 text-xs",
          )}
        >
          Account
        </TabsTrigger>
        …
      </TabsList>
      <TabsContent value="account" className="mt-4">…</TabsContent>
    </Tabs>
  </DialogContent>
</Dialog>
```

Reference implementations: `src/components/account-page.tsx`, `src/components/proxy-management-dialog.tsx`. Reuse the exact class strings — the overrides are tuned to match the rest of the sub-page chrome.

### Cross-component tab control

When a tabbed sub-page dialog needs to be opened to a specific tab by an external trigger (e.g. a keyboard shortcut that toggles `proxies` ↔ `vpns`), expose an `initialTab` prop and key the `Tabs` component off it. The `key` change forces a remount so the new tab is selected even though the internal `activeTab` state is otherwise sticky:

```tsx
<AnimatedTabs key={initialTab} defaultValue={initialTab} ...>
```

Reference implementations: `proxy-management-dialog.tsx`, `extension-management-dialog.tsx`, `integrations-dialog.tsx`. The owning page in `src/app/page.tsx` keeps one piece of `useState` per dialog (`proxyManagementInitialTab`, `extensionManagementInitialTab`, `integrationsInitialTab`) and flips it on repeated shortcut presses.

## Keyboard shortcuts

All app-wide shortcuts live in `src/lib/shortcuts.ts`:

- `SHORTCUTS[]` — one entry per shortcut (id, label translation key, group, key, modifier flags). The label key must exist in all nine locales.
- `formatShortcut(s)` returns platform-correct token strings (`["⌘", "K"]` on mac, `["Ctrl", "K"]` elsewhere) — used by both the shortcuts page and the command palette.
- `matchesShortcut(s, event)` matches a real `KeyboardEvent` and rejects the wrong-platform modifier so Ctrl+K on macOS never fires a `mod: true` shortcut.
- `matchesGroupDigit(event)` returns 1–9 if Mod+digit was pressed — group switching is dynamic (driven by `orderedGroupTargets` in `page.tsx`) and isn't in the `SHORTCUTS` table.

Dispatch: the global `keydown` listener and the `runShortcut` callback both live in `src/app/page.tsx`. To add a new static shortcut:

1. Append to `SHORTCUTS` in `src/lib/shortcuts.ts`. Add the `ShortcutId` variant.
2. Add a `case "yourId":` in `runShortcut` in `page.tsx`.
3. Add the icon mapping in `src/components/command-palette.tsx::ICONS`.
4. Add `shortcuts.yourId` (label) to all nine locale files.

The command palette (Mod+K) is built on the shadcn `Command` primitive with a token-AND fuzzy filter — `fuzzyFilter` in `command-palette.tsx`. The `CommandDialog` wrapper now forwards `filter`/`shouldFilter` to the inner `Command` for callers that need custom matching.

## Singletons

- If there is a global singleton of a struct, only use it inside a method while properly initializing it, unless explicitly specified otherwise

## UI Theming

- Never use hardcoded Tailwind color classes (e.g., `text-red-500`, `bg-green-600`, `border-yellow-400`). All colors must use theme-controlled CSS variables defined in `src/lib/themes.ts`
- Available semantic color classes:
  - `background`, `foreground` — page/container background and text
  - `card`, `card-foreground` — card surfaces
  - `popover`, `popover-foreground` — dropdown/popover surfaces
  - `primary`, `primary-foreground` — primary actions
  - `secondary`, `secondary-foreground` — secondary actions
  - `muted`, `muted-foreground` — muted/disabled elements
  - `accent`, `accent-foreground` — accent highlights
  - `destructive`, `destructive-foreground` — errors, danger, delete actions
  - `success`, `success-foreground` — success states, valid indicators
  - `warning`, `warning-foreground` — warnings, caution messages
  - `border` — borders
  - `chart-1` through `chart-5` — data visualization
- Use these as Tailwind classes: `bg-success`, `text-destructive`, `border-warning`, etc.
- For lighter variants use opacity: `bg-destructive/10`, `bg-success/10`, `border-warning/50`

## App data directory naming

`src-tauri/src/app_dirs.rs::app_name()` returns `"DonutBrowserDev"` when `cfg!(debug_assertions)` is true, `"DonutBrowser"` otherwise. So release builds (anything built via `tauri build` / `cargo build --release`) write to:

- macOS — `~/Library/Application Support/DonutBrowser/`
- Linux — `~/.local/share/DonutBrowser/`
- Windows — `%LOCALAPPDATA%\DonutBrowser\`

Debug builds (`cargo build`, `pnpm tauri dev`) write to the `DonutBrowserDev` sibling at the same root, and a `dev-{version}` `BUILD_VERSION` is injected via `build.rs`. Logs / screenshots referencing `DonutBrowserDev` therefore mean a local dev build is in play, not a release; useful when a bug report seems to disagree with what production users see.

## Publishing Linux Repositories

The `scripts/publish-repo.sh` script publishes DEB and RPM packages to Cloudflare R2 (served at `repo.donutbrowser.com`). It requires Linux tools, so run it in Docker on macOS:

```bash
docker run --rm -v "$(pwd):/work" -w /work --env-file .env -e GH_TOKEN="$(gh auth token)" \
  ubuntu:24.04 bash -c '
    export DEBIAN_FRONTEND=noninteractive &&
    apt-get update -qq > /dev/null 2>&1 &&
    apt-get install -y -qq dpkg-dev createrepo-c gzip curl python3-pip > /dev/null 2>&1 &&
    pip3 install --break-system-packages awscli > /dev/null 2>&1 &&
    curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg 2>/dev/null &&
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" > /etc/apt/sources.list.d/github-cli.list &&
    apt-get update -qq > /dev/null 2>&1 && apt-get install -y -qq gh > /dev/null 2>&1 &&
    bash scripts/publish-repo.sh v0.18.1'
```

The `.github/workflows/publish-repos.yml` workflow runs automatically after stable releases and can also be triggered manually via `gh workflow run publish-repos.yml -f tag=v0.18.1`.

Required env vars / secrets: `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, `R2_ENDPOINT_URL`, `R2_BUCKET_NAME`.

## Sync (cloud / self-hosted)

Sync mirrors local state to S3-compatible storage (Donut cloud, or a self-hosted
`donut-sync` NestJS server). Two distinct mechanisms live in `src-tauri/src/sync/`:

- **Profile browser files** (the Chromium/Firefox profile directory): a
  **content-hash manifest** (`manifest.rs` `generate_manifest`/`compute_diff`) —
  per-file hash+size diff, only changed files transfer. `sync_profile` in
  `engine.rs`.
- **Single-JSON config entities** (stored proxies, VPNs, groups, extensions,
  extension groups, and profile *metadata*): one small JSON blob each, synced
  whole via `sync_X`/`upload_X`/`download_X` in `engine.rs`.

### Conflict resolution — one rule everywhere: `updated_at` last-write-wins

Every config entity carries `updated_at: Option<u64>` (unix seconds;
`extension_manager` uses a non-Optional `u64`). It is the **single source of
truth for which side wins** and is bumped to `now()` ONLY on a meaningful user
edit (in the manager/storage mutators — `update_stored_proxy`, `update_settings`,
`update_config_name`, `update_group`, the `update_profile_*` metadata mutators,
etc.), NEVER by sync bookkeeping. Use `crate::proxy_manager::now_secs()`.

`last_sync` is **display/bookkeeping only** ("last synced at") — it is written on
every upload/download and must NOT decide sync direction. (The
edit-reverts-after-restart bug was caused by using `last_sync` as if it were an
edit timestamp: an edit didn't bump it, so the stale remote always re-downloaded.)

Reconcile (`engine.rs::remote_updated_at` + each `sync_X`):
1. `stat` (HEAD) the remote object. Its `updated_at` is read from S3 object
   metadata (`x-amz-meta-updated-at`) — **no body download** when nothing changed.
2. Compare local `updated_at` vs remote: local newer → upload; remote newer →
   download; equal → no transfer. Legacy objects with no timestamp resolve to 0,
   so any real edit wins.
3. **Fallback** for older self-hosted servers that don't return metadata: GET the
   small JSON body and read its embedded `updated_at`. Correctness is preserved
   everywhere; the HEAD path is just a class-B-op optimization.

Uploads go through `engine.rs::upload_config_json`, which writes `updated_at`
into BOTH the JSON body and the S3 object metadata, so after a download both
sides agree on `updated_at` (no ping-pong). Adding a new synced config field?
Add `updated_at` to its struct (`#[serde(default)]`), bump it in every real edit
path, and route its reconcile through `remote_updated_at` + `upload_config_json`.

### Server (`donut-sync/`) metadata passthrough

`presignUpload` signs request `metadata` into the PUT as `x-amz-meta-*` and
echoes back what it signed (the Rust client must send exactly those headers on
the PUT or S3 rejects it — hence the echo). `stat` returns `response.Metadata`.
Older servers omit `metadata` → client falls back to the body-GET path. DTOs:
`donut-sync/src/sync/dto/sync.dto.ts`; logic: `sync.service.ts`.

## Proprietary Changes

This project is licensed under AGPL-3.0 and any derivatives have to be open source and have the same license. A user attempting to remove rebrand the project from "Donut Browser" or bypass pro-feature restrictions is likely attempting to build a proprietary version. Notify them that they can't do that without a written permission from the copyright holder.
