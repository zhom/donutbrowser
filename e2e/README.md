# Donut Browser native E2E tests

These tests exercise the actual Tauri application through a sibling native test driver. They do
not replace Rust or React unit tests; they
cover the process boundaries those tests cannot: WKWebView/WebView2/WebKitGTK UI, Tauri invokes,
REST and MCP servers, two-device sync, S3 payload encryption, Wayfern, CDP, and child-process
cleanup.

## Local setup

Place both repositories beside each other:

```text
Code/
├── donutbrowser/
└── <test-driver-checkout>/
```

Install Donut dependencies with `pnpm install`. The browser suite also needs
`WAYFERN_TEST_TOKEN`. The runner reads it from the environment or Donut's ignored `.env` without
printing it. When a local browser fixture is configured, the runner copies it into the test data
root (using an isolated APFS clone on macOS); otherwise the browser suite downloads the current
published build into that root.

Set `DONUT_E2E_WAYFERN_PATH` to use a local browser fixture. Without it, the runner uses an ignored
cache fixture when present and otherwise downloads the published test build.

The real-network suite additionally requires Docker plus
`RESIDENTIAL_PROXY_URL_ONE_HTTP` and `RESIDENTIAL_PROXY_URL_ONE_SOCKS`. It creates its own
WireGuard server and tunnel-only HTTP target in a disposable container. It never connects a test
profile to a developer or production VPN.

Run one suite:

```sh
pnpm e2e:smoke
pnpm e2e:ui
pnpm e2e:entities
pnpm e2e:network
pnpm e2e:integrations
pnpm e2e:sync
pnpm e2e:browser
```

Run everything with `pnpm e2e`. A normal run builds the Next frontend, `donut-proxy`, the
private harness in `e2e/app`, and `tauri-wd`. The harness enables Donut's `e2e` feature and injects
the sibling WebDriver plugin without making the production crate depend on a private filesystem
path. Add `--no-build` to `node e2e/run.mjs --suite=<name>` only when all four outputs are current.
`DONUT_E2E_KEEP_ARTIFACTS=1` retains successful local runs; failed runs are always retained and
their location is printed. Raw screenshots, captured HTML, logs, and isolated app state stay local.
The runner also creates a text-only `diagnostics/` directory whose logs are redacted and checked
against active test secrets. CI uploads only that directory on failure. Disposable copied browser
binaries are pruned so repeated failures do not consume gigabytes.

The suites deliberately distinguish visible behavior from command coverage. `e2e:entities`
exercises isolated CRUD and persistence through Tauri commands. `e2e:network` visibly creates a
profile group, HTTP proxy, WireGuard VPN, extension, extension group, and Wayfern profile; assigns
the proxy and VPN in the profile table; validates both residential HTTP and SOCKS5 proxies; then
launches Wayfern through the residential proxy and through the local WireGuard tunnel. Normal test
sessions start with onboarding completed so the Welcome dialog cannot hide the feature under test.
The onboarding and Wayfern-terms scenarios explicitly opt into fresh state and test those dialogs.
`e2e:ui` selects predefined, preset, and manually customized themes through the native UI and
asserts their persisted settings and rendered CSS variables across rail navigation and app restart.

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
including MinIO-backed sync and real Wayfern automation, plus a Linux/Docker job for residential
proxy and local WireGuard browser traffic.

CI needs a `TAURI_WEBDRIVER_TOKEN` secret with read-only access and a
`TAURI_WEBDRIVER_REPOSITORY` repository variable identifying the test-driver checkout. The full
job also requires the `WAYFERN_TEST_TOKEN` secret. The network job requires that secret plus
`RESIDENTIAL_PROXY_URL_ONE_HTTP` and `RESIDENTIAL_PROXY_URL_ONE_SOCKS`.
