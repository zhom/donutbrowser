# Donut Browser native E2E tests

These tests exercise the actual Tauri application through the private
`../tauri-cross-platform-webdriver/` driver. They do not replace Rust or React unit tests; they
cover the process boundaries those tests cannot: WKWebView/WebView2/WebKitGTK UI, Tauri invokes,
REST and MCP servers, two-device sync, S3 payload encryption, Wayfern, CDP, and child-process
cleanup.

## Local setup

Place both repositories beside each other:

```text
Code/
├── donutbrowser/
└── tauri-cross-platform-webdriver/
```

Install Donut dependencies with `pnpm install`. The browser suite also needs
`WAYFERN_TEST_TOKEN`. The runner reads it from the environment, Donut's `.env`, or
`../wayfern-test/.env` without printing it. If `../wayfern-test/test_extracted_app` contains a
Wayfern build, the runner links/copies it into the test data root; otherwise the browser suite
downloads the current published build into that root.

Run one suite:

```sh
pnpm e2e:smoke
pnpm e2e:ui
pnpm e2e:entities
pnpm e2e:integrations
pnpm e2e:sync
pnpm e2e:browser
```

Run everything with `pnpm e2e`. A normal run builds the Next frontend, `donut-proxy`, the
`e2e`-feature app, and `tauri-wd`. Add `--no-build` to `node e2e/run.mjs --suite=<name>` only when
all four outputs are current. `DONUT_E2E_KEEP_ARTIFACTS=1` retains successful runs; failed runs are
always retained and their location is printed.

## Isolation contract

Each app session receives a unique root under the operating-system test temp directory. The
runner redirects:

- Donut data, cache, and logs with `DONUTBROWSER_DATA_ROOT`;
- `HOME`, `USERPROFILE`, `CFFIXED_USER_HOME`, XDG paths, `APPDATA`, and `LOCALAPPDATA`;
- `TMPDIR`, `TMP`, and `TEMP`;
- the Tauri WebView store (incognito for WKWebView, whose persistent data-directory API is not
  honored);
- all REST, MCP, WebDriver, fixture, MinIO, and sync-server ports;
- each sync test to a new MinIO bucket and random token.

The E2E feature suppresses automatic updater/download traffic, but explicit browser tests still
exercise published Wayfern downloads when no local fixture exists. Entitlement fallback from
`WAYFERN_TEST_TOKEN` exists only in the feature-gated test binary. Production builds never include
the WebDriver plugin or this fallback.

## CI

`.github/workflows/app-e2e.yml` runs smoke tests on macOS, Linux/Xvfb, and Windows for pull
requests. Pushes to `main`, weekly schedules, and manual runs execute the full macOS suite,
including MinIO-backed sync and real Wayfern automation.

Because the WebDriver repository is private, CI needs a `TAURI_WEBDRIVER_TOKEN` secret with
read-only access. `TAURI_WEBDRIVER_REPOSITORY` may override its default
`zhom/tauri-cross-platform-webdriver` repository name. The full job also requires the
`WAYFERN_TEST_TOKEN` secret.
