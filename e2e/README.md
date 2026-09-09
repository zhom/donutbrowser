# Donut Browser native E2E tests

These tests exercise the actual Tauri application through the published
[`tauri-wd`](https://crates.io/crates/tauri-wd) native test driver. They do
not replace Rust or React unit tests; they
cover the process boundaries those tests cannot: WKWebView/WebView2/WebKitGTK UI, Tauri invokes,
REST and MCP servers, two-device sync, S3 payload encryption, Wayfern, CDP, and child-process
cleanup.

## Local setup

Install Donut dependencies with `pnpm install`. The runner installs the driver itself with
`cargo install`, so a working Rust toolchain is the only extra requirement. The browser suite also
needs
`WAYFERN_TEST_TOKEN`. The runner reads it from the environment or Donut's ignored `.env` without
printing it. The browser suites always run the newest published Wayfern build. The download is
saved as an ignored cache fixture under `.cache/e2e-wayfern-fixture`, which the runner copies into
the test data root (using an isolated APFS clone on macOS) on later runs; a cached fixture holding
any other version is replaced before the suite uses it, so the cache can never keep an old browser
under test.

Set `DONUT_E2E_WAYFERN_PATH` to pin an explicit local bundle instead, for example a browser built
from source. A pinned bundle is used as given, without the published-version check.

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

Run everything with `pnpm e2e`. A normal run builds the Next frontend, `donut-proxy`, and the
harness in `e2e/app`, then installs the `tauri-wd` CLI into the ignored `e2e/.driver` root when the
version pinned by `e2e/app/Cargo.toml` is not already there. The harness enables Donut's `e2e`
feature and injects the WebDriver plugin so the production crate never depends on it. Both the
plugin and the CLI come from the same pinned crates.io release, so they cannot drift apart. Bump
the pin in `e2e/app/Cargo.toml` to move to a newer driver. Every suite runs the Donut window
headless (on macOS the window is transparent, click-through and never focused; elsewhere it is
hidden), so a run never pops a window or steals focus. Set `DONUT_E2E_HEADED=1` to watch the
window while debugging a failure; Wayfern browsers launched by a test are separate processes and
show their own windows unless the test asks for a headless launch.
Add `--no-build` to
`node e2e/run.mjs --suite=<name>` only when all four outputs are current.
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
exercise published Wayfern downloads whenever the cache fixture is missing or holds a different
version than the published build. Entitlement fallback from
`WAYFERN_TEST_TOKEN` exists only in the feature-gated test binary. Production builds never include
the WebDriver plugin or this fallback.

## CI

`.github/workflows/app-e2e.yml` runs smoke tests on macOS, Linux/Xvfb, and Windows for pull
requests. Pushes to `main`, weekly schedules, and manual runs execute the full macOS suite,
including MinIO-backed sync and real Wayfern automation, plus a Linux/Docker job for residential
proxy and local WireGuard browser traffic.

Every job restores the compiled driver from an `actions/cache` entry keyed by `e2e/app/Cargo.lock`,
the same file the runner reads the version from, so only a driver bump pays for a rebuild. The full job requires the
`WAYFERN_TEST_TOKEN` secret. The network job requires that secret plus
`RESIDENTIAL_PROXY_URL_ONE_HTTP` and `RESIDENTIAL_PROXY_URL_ONE_SOCKS`.
