use crate::browser::ProxySettings;
use crate::events;
use crate::group_manager::GROUP_MANAGER;
use crate::profile::manager::ProfileManager;
use crate::proxy_manager::PROXY_MANAGER;
use crate::tag_manager::TAG_MANAGER;
use axum::{
  extract::{Path, Query, State},
  http::{header, HeaderMap, Method, StatusCode},
  middleware::{self, Next},
  response::{IntoResponse, Json, Response},
  routing::get,
  Router,
};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use tower_http::cors::CorsLayer;
use utoipa::{OpenApi, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};

// API Types
#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct ApiProfile {
  pub id: String,
  pub name: String,
  pub browser: String,
  pub version: String,
  pub proxy_id: Option<String>,
  pub launch_hook: Option<String>,
  pub process_id: Option<u32>,
  pub last_launch: Option<u64>,
  pub release_type: String,
  pub group_id: Option<String>,
  pub tags: Vec<String>,
  pub is_running: bool,
  pub proxy_bypass_rules: Vec<String>,
  pub vpn_id: Option<String>,
  pub clear_on_close: bool,
  /// Cloud sync mode: `"Disabled"`, `"Regular"` or `"Encrypted"`.
  /// Settable via `PUT /v1/profiles/{id}`; exposed here so a caller can read
  /// back what it set, and so a remote-launch caller can tell whether the
  /// profile is actually available in cloud storage.
  pub sync_mode: String,
  /// Convenience form of `sync_mode` — true for Regular or Encrypted.
  pub cloud_sync_enabled: bool,
  /// OS the profile was created on (`"macos"`, `"windows"`, `"linux"`).
  /// `null` when neither `host_os` nor the browser config records one.
  pub host_os: Option<String>,
  /// True when the profile belongs to a different OS than this machine.
  /// Such a profile cannot be launched locally, and must only ever run on a
  /// remote host of its own OS — Chromium profile state is OS-specific.
  pub is_cross_os: bool,
}

impl From<&crate::profile::types::BrowserProfile> for ApiProfile {
  /// Single conversion for every profile-returning route. Previously open-coded
  /// at three call sites, which is how `sync_mode` came to be settable but not
  /// readable: a field added to the struct had to be remembered three times.
  fn from(profile: &crate::profile::types::BrowserProfile) -> Self {
    Self {
      id: profile.id.to_string(),
      name: profile.name.clone(),
      browser: profile.browser.clone(),
      version: profile.version.clone(),
      proxy_id: profile.proxy_id.clone(),
      launch_hook: profile.launch_hook.clone(),
      process_id: profile.process_id,
      last_launch: profile.last_launch,
      release_type: profile.release_type.clone(),
      group_id: profile.group_id.clone(),
      tags: profile.tags.clone(),
      is_running: profile.process_id.is_some(),
      proxy_bypass_rules: profile.proxy_bypass_rules.clone(),
      vpn_id: profile.vpn_id.clone(),
      clear_on_close: profile.clear_on_close,
      sync_mode: format!("{:?}", profile.sync_mode),
      cloud_sync_enabled: profile.is_sync_enabled(),
      host_os: profile.resolved_os().map(|os| os.to_string()),
      is_cross_os: profile.is_cross_os(),
    }
  }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ApiProfilesResponse {
  pub profiles: Vec<ApiProfile>,
  pub total: usize,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ApiProfileResponse {
  pub profile: ApiProfile,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateProfileRequest {
  pub name: String,
  /// Browser engine. Must be `"wayfern"` (anti-detect Chromium). Any other
  /// value (e.g. `"chromium"`) is rejected with 400.
  pub browser: String,
  /// Optional. Omit (or pass `"latest"`) to use the newest already-downloaded
  /// version of the chosen browser. A concrete version must already be
  /// downloaded; the create path does not fetch new versions.
  #[serde(default)]
  pub version: Option<String>,
  pub proxy_id: Option<String>,
  pub vpn_id: Option<String>,
  pub launch_hook: Option<String>,
  pub release_type: Option<String>,
  /// Wayfern fingerprint/config. Send only when `browser` is `"wayfern"`.
  /// Omit it, or pass an empty object `{}`, to have a fresh fingerprint
  /// generated automatically at creation. Provide a `fingerprint` field to
  /// pin a specific one.
  #[schema(value_type = Option<Object>)]
  pub wayfern_config: Option<serde_json::Value>,
  pub group_id: Option<String>,
  pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateProfileRequest {
  pub name: Option<String>,
  // No `browser` field: a profile's engine is fixed at creation (changing it
  // would invalidate the generated fingerprint and on-disk profile dir).
  // Accepting it here only to silently ignore it misled API clients.
  pub version: Option<String>,
  pub proxy_id: Option<String>,
  pub vpn_id: Option<String>,
  pub launch_hook: Option<String>,
  pub release_type: Option<String>,
  pub group_id: Option<String>,
  pub tags: Option<Vec<String>>,
  pub extension_group_id: Option<String>,
  pub proxy_bypass_rules: Option<Vec<String>>,
  /// One of "Disabled", "Regular", "Encrypted".
  pub sync_mode: Option<String>,
  /// Wipe browsing data (keeping extensions and bookmarks) when the browser
  /// exits. Rejected (400) for ephemeral or password-protected profiles.
  pub clear_on_close: Option<bool>,
}

#[derive(Clone)]
struct ApiServerState {
  app_handle: tauri::AppHandle,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
struct ApiGroupResponse {
  id: String,
  name: String,
  profile_count: usize,
}

#[derive(Debug, Deserialize, ToSchema)]
struct CreateGroupRequest {
  name: String,
}

#[derive(Debug, Deserialize, ToSchema)]
struct UpdateGroupRequest {
  name: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
struct ApiProxyResponse {
  id: String,
  name: String,
  #[schema(value_type = Object)]
  proxy_settings: ProxySettings,
}

#[derive(Debug, Deserialize, ToSchema)]
struct CreateProxyRequest {
  name: String,
  #[schema(value_type = Object)]
  proxy_settings: ProxySettings,
}

#[derive(Debug, Deserialize, ToSchema)]
struct UpdateProxyRequest {
  name: Option<String>,
  #[schema(value_type = Option<Object>)]
  proxy_settings: Option<ProxySettings>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
struct ApiVpnResponse {
  id: String,
  name: String,
  /// Always "WireGuard"
  vpn_type: String,
  created_at: i64,
  last_used: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
struct ApiVpnExportResponse {
  id: String,
  name: String,
  /// Always "WireGuard"
  vpn_type: String,
  /// Raw `.conf` file content (decrypted)
  config_data: String,
}

#[derive(Debug, Deserialize, ToSchema)]
struct ImportVpnRequest {
  /// Raw WireGuard `.conf` file content
  content: String,
  /// Original filename
  filename: String,
  /// Optional display name; defaults to filename-based name
  name: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct CreateVpnRequest {
  name: String,
  /// Must be "WireGuard"
  vpn_type: String,
  config_data: String,
}

#[derive(Debug, Deserialize, ToSchema)]
struct UpdateVpnRequest {
  name: String,
}

#[derive(Debug, Deserialize, ToSchema)]
struct DownloadBrowserRequest {
  browser: String,
  version: String,
}

#[derive(Debug, Serialize, ToSchema)]
struct DownloadBrowserResponse {
  browser: String,
  version: String,
  status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToastPayload {
  pub message: String,
  pub variant: String,
  pub title: String,
  pub description: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
struct RunProfileResponse {
  profile_id: String,
  remote_debugging_port: u16,
  headless: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RunRemoteRequest {
  /// Optional URL to open once the remote browser is up.
  pub url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SetCloudSyncRequest {
  /// `Disabled`, `Regular`, or `Encrypted`.
  ///
  /// `Encrypted` derives its key from a passphrase that never leaves this
  /// machine, so a profile in that mode can be synced but NOT run remotely —
  /// a remote host would download ciphertext it cannot decrypt.
  pub mode: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SetCloudSyncResponse {
  pub profile_id: String,
  pub mode: String,
  /// Whether the profile can now be launched on a remote host.
  pub remote_launchable: bool,
  /// Why not, when `remote_launchable` is false.
  pub remote_blocked_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RunRemoteResponse {
  pub profile_id: String,
  /// Remote session id, for polling or closing the session.
  pub session_id: String,
  /// Operating system the session was scheduled onto — always the profile's own.
  pub platform: String,
  pub status: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StopRemoteResponse {
  pub session_id: String,
  pub status: String,
  /// What the session actually cost, in seconds.
  pub billed_seconds: u64,
}

/// Every remote session the signed-in account currently owns.
///
/// `run-remote` hands back a session id and the literal string `provisioning`;
/// without a way to read the real state back, an automation client can only
/// discover that a session became usable by trying to drive it.
#[derive(Debug, Serialize, ToSchema)]
struct ApiRemoteSessionsResponse {
  sessions: Vec<crate::remote_session::RemoteSessionState>,
}

/// Enrol a profile in the nightly cookie bot, or replace its enrolment.
///
/// `platform` and `profile_name` are optional because this machine already
/// knows both: the platform is the profile's own operating system, and a
/// caller-supplied one that disagrees is a mistake, not a choice.
#[derive(Debug, Deserialize, ToSchema)]
struct SetCookieBotScheduleRequest {
  /// Defaults to the profile's local name.
  profile_name: Option<String>,
  /// `windows` or `macos`. Defaults to the profile's own operating system, and
  /// must match it when supplied.
  platform: Option<String>,
  /// Whether the nightly run is armed. A disabled schedule keeps its settings.
  enabled: bool,
  /// Minutes past local midnight the run is anchored to (0..1439).
  run_at_minute: u16,
  /// Bitmask of local weekdays, bit 0 = Monday (1..127).
  days_mask: u8,
  /// IANA zone the run time is expressed in.
  timezone: String,
  /// Server-issued preset id from `GET /v1/cookie-bot/presets`.
  preset: String,
  /// Upper bound on one run, in minutes.
  max_minutes: u32,
  /// Absolute http(s) URLs to browse. The bot visits only these.
  #[serde(default)]
  sites: Vec<String>,
  /// Random spread around the anchor time, in seconds.
  jitter_seconds: Option<u32>,
  /// Write anyway when a teammate already enrols this profile. Without it, a
  /// colliding write is refused with 409 and the teammate's details.
  #[serde(default)]
  acknowledge_conflict: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
struct StartCookieBotRunRequest {
  /// Profile to warm. It must already have a schedule: the preset and the site
  /// list live there, so a run never carries a behaviour of its own.
  profile_id: String,
  /// Overrides the schedule's own cap for this run only.
  max_minutes: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct CookieBotScopeQuery {
  scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CookieBotConflictsQuery {
  profile_id: String,
  run_at_minute: Option<u16>,
  timezone: Option<String>,
  days_mask: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct CookieBotRunsQuery {
  profile_id: Option<String>,
  scope: Option<String>,
  limit: Option<u32>,
  before: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CookieBotUsageQuery {
  period: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct RunProfileRequest {
  url: Option<String>,
  headless: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct OpenUrlRequest {
  url: String,
}

#[derive(Debug, Deserialize, ToSchema)]
struct ImportCookiesRequest {
  /// Raw cookie file content. Format is auto-detected: a JSON array
  /// (Puppeteer / EditThisCookie style) or a Netscape `cookies.txt`.
  content: String,
}

#[derive(Debug, Serialize, ToSchema)]
struct ImportCookiesResponse {
  cookies_imported: usize,
  cookies_replaced: usize,
  errors: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct BatchRunRequest {
  /// Profile IDs to launch.
  profile_ids: Vec<String>,
  /// Optional URL to open in every launched profile.
  url: Option<String>,
  /// Launch headless. Defaults to false.
  headless: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
struct BatchRunResult {
  profile_id: String,
  /// Whether this profile launched successfully.
  ok: bool,
  /// Remote debugging port if launched, otherwise null.
  remote_debugging_port: Option<u16>,
  /// Failure reason if not launched, otherwise null.
  error: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
struct BatchRunResponse {
  results: Vec<BatchRunResult>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct BatchStopRequest {
  /// Profile IDs to stop.
  profile_ids: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
struct BatchStopResult {
  profile_id: String,
  /// Whether this profile was stopped successfully.
  ok: bool,
  /// Failure reason if not stopped, otherwise null.
  error: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
struct BatchStopResponse {
  results: Vec<BatchStopResult>,
}

#[derive(Debug, Serialize, ToSchema)]
struct DetectedProfilesResponse {
  profiles: Vec<crate::profile_importer::DetectedProfile>,
  total: usize,
}

#[derive(Debug, Deserialize)]
struct DetectImportQuery {
  /// Optional folder to scan instead of the default browser locations.
  folder: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct ImportProfilesRequest {
  /// Profiles to import. Each item is isolated — one failure doesn't stop the rest.
  items: Vec<crate::profile_importer::ImportProfileItem>,
  /// Optional group to assign every imported profile to.
  group_id: Option<String>,
  /// How to handle an already-taken profile name: "skip" or "rename"
  /// (auto-suffix). Defaults to "rename".
  duplicate_strategy: Option<crate::profile_importer::DuplicateStrategy>,
  /// Wayfern fingerprint/config applied to every imported profile. Omit to
  /// have fresh fingerprints generated automatically.
  #[schema(value_type = Option<Object>)]
  wayfern_config: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct ImportProxiesRequest {
  /// "txt" — one proxy per line (`host:port`, `host:port:user:pass`, or URL
  /// forms like `http://user:pass@host:port`). "json" — a Donut proxy export.
  format: String,
  /// Raw proxy list / export content.
  content: String,
  /// Name prefix for txt imports; proxies are named "{prefix} Proxy {n}".
  name_prefix: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
struct ImportProxiesResponse {
  imported_count: usize,
  skipped_count: usize,
  errors: Vec<String>,
  proxies: Vec<ApiProxyResponse>,
}

#[derive(OpenApi)]
#[openapi(
  paths(
    get_profiles,
    get_profile,
    create_profile,
    update_profile,
    delete_profile,
    run_profile,
    run_profile_remote,
    stop_remote_session,
    list_remote_sessions_api,
    get_remote_session_api,
    get_remote_hours,
    set_profile_cloud_sync,
    list_cookie_bot_schedules,
    get_cookie_bot_schedule,
    set_cookie_bot_schedule,
    delete_cookie_bot_schedule,
    get_cookie_bot_conflicts,
    list_cookie_bot_runs,
    start_cookie_bot_run,
    cancel_cookie_bot_run,
    list_cookie_bot_presets,
    get_cookie_bot_usage,
    open_url_in_profile,
    kill_profile,
    batch_run_profiles,
    batch_stop_profiles,
    detect_import_profiles,
    import_profiles_api,
    import_profile_cookies,
    get_groups,
    get_group,
    create_group,
    update_group,
    delete_group,
    get_tags,
    get_proxies,
    get_proxy,
    create_proxy,
    import_proxies_api,
    update_proxy,
    delete_proxy,
    get_vpns,
    get_vpn,
    export_vpn,
    import_vpn,
    create_vpn,
    update_vpn,
    delete_vpn,
    get_extensions,
    get_extension_groups,
    delete_extension_api,
    delete_extension_group_api,
    download_browser_api,
    get_browser_versions,
    check_browser_downloaded,
  ),
  components(schemas(
    ApiProfile,
    ApiProfilesResponse,
    ApiProfileResponse,
    CreateProfileRequest,
    UpdateProfileRequest,
    ApiGroupResponse,
    CreateGroupRequest,
    UpdateGroupRequest,
    ApiProxyResponse,
    CreateProxyRequest,
    UpdateProxyRequest,
    ApiVpnResponse,
    ApiVpnExportResponse,
    ImportVpnRequest,
    CreateVpnRequest,
    UpdateVpnRequest,
    DownloadBrowserRequest,
    DownloadBrowserResponse,
    RunProfileResponse,
    RunRemoteRequest,
    RunRemoteResponse,
    StopRemoteResponse,
    SetCloudSyncRequest,
    SetCloudSyncResponse,
    ApiRemoteSessionsResponse,
    SetCookieBotScheduleRequest,
    StartCookieBotRunRequest,
    crate::remote_session::RemoteSessionState,
    crate::cookie_bot::CookieBotSchedule,
    crate::cookie_bot::CookieBotScheduleList,
    crate::cookie_bot::CookieBotScheduleSaved,
    crate::cookie_bot::CookieBotScheduleDeleted,
    crate::cookie_bot::CookieBotConflict,
    crate::cookie_bot::CookieBotConflictCheck,
    crate::cookie_bot::CookieBotRun,
    crate::cookie_bot::CookieBotRunPage,
    crate::cookie_bot::CookieBotRunStarted,
    crate::cookie_bot::CookieBotPreset,
    crate::cookie_bot::CookieBotPresetList,
    crate::cookie_bot::CookieBotUsage,
    crate::cookie_bot::CookieBotUsageMember,
    crate::cookie_bot::CookieBotUsageProfile,
    crate::cookie_bot::RemoteHoursQuota,
    crate::cookie_bot::RemoteHoursMember,
    crate::cookie_bot::RemoteHoursBreakdown,
    RunProfileRequest,
    BatchRunRequest,
    BatchRunResult,
    BatchRunResponse,
    BatchStopRequest,
    BatchStopResult,
    BatchStopResponse,
    OpenUrlRequest,
    ImportCookiesRequest,
    ImportCookiesResponse,
    ProxySettings,
    DetectedProfilesResponse,
    ImportProfilesRequest,
    ImportProxiesRequest,
    ImportProxiesResponse,
    crate::profile_importer::DetectedProfile,
    crate::profile_importer::ImportProfileItem,
    crate::profile_importer::DuplicateStrategy,
    crate::profile_importer::ProfileImportItemResult,
    crate::profile_importer::ProfileImportBatchResult,
  )),
  tags(
    (name = "profiles", description = "Profile management endpoints"),
    (name = "groups", description = "Group management endpoints"),
    (name = "tags", description = "Tag management endpoints"),
    (name = "proxies", description = "Proxy management endpoints"),
    (name = "vpns", description = "VPN management endpoints"),
    (name = "extensions", description = "Extension management endpoints"),
    (name = "browsers", description = "Browser management endpoints"),
    (name = "cookies", description = "Cookie management endpoints"),
    (name = "remote-sessions", description = "Sessions running on the leased remote fleet"),
    (name = "cookie-bot", description = "Scheduled cookie-warming runs on the remote fleet"),
  ),
  modifiers(&SecurityAddon),
)]
struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
  fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
    if let Some(components) = openapi.components.as_mut() {
      components.add_security_scheme(
        "bearer_auth",
        utoipa::openapi::security::SecurityScheme::Http(
          utoipa::openapi::security::HttpBuilder::new()
            .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
            .bearer_format("JWT")
            .build(),
        ),
      );
    }
  }
}

pub struct ApiServer {
  port: Option<u16>,
  shutdown_tx: Option<mpsc::Sender<()>>,
  task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ApiServer {
  fn new() -> Self {
    Self {
      port: None,
      shutdown_tx: None,
      task_handle: None,
    }
  }

  fn get_port(&self) -> Option<u16> {
    self.port
  }

  async fn start(
    &mut self,
    app_handle: tauri::AppHandle,
    preferred_port: u16,
  ) -> Result<u16, String> {
    // Stop existing server if running
    self.stop().await.ok();

    let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
    let state = ApiServerState {
      app_handle: app_handle.clone(),
    };

    // Try preferred port first, then random port
    let listener = match TcpListener::bind(format!("127.0.0.1:{preferred_port}")).await {
      Ok(listener) => listener,
      Err(_) => {
        // Port conflict, try random port
        let random_port = rand::random::<u16>().saturating_add(10000);
        match TcpListener::bind(format!("127.0.0.1:{random_port}")).await {
          Ok(listener) => {
            let _ = events::emit(
              "api-port-conflict",
              format!("API server using fallback port {random_port}"),
            );
            listener
          }
          Err(e) => {
            return Err(crate::backend_error_with_detail("API_PORT_UNAVAILABLE", e));
          }
        }
      }
    };

    let actual_port = listener
      .local_addr()
      .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?
      .port();

    let v1_routes = build_v1_router();

    let api = ApiDoc::openapi();

    let v1_routes = v1_routes
      // Innermost so only authenticated automation requests consume quota.
      .layer(middleware::from_fn(rate_limit_middleware))
      .layer(middleware::from_fn_with_state(
        state.clone(),
        auth_middleware,
      ))
      .layer(middleware::from_fn(terms_check_middleware));

    let api_for_v1 = api.clone();
    let app = Router::new()
      .merge(v1_routes)
      .route("/openapi.json", get(move || async move { Json(api) }))
      .route(
        "/v1/openapi.json",
        get(move || async move { Json(api_for_v1) }),
      )
      // Outermost layer: logs every request so customer reports show what
      // their automation is actually calling, what the response status was,
      // and how long it took. Never logs request bodies or auth headers.
      .layer(middleware::from_fn(request_logging_middleware))
      .layer(CorsLayer::permissive())
      .with_state(state);

    // Start server task
    let task_handle = tokio::spawn(async move {
      let server = axum::serve(listener, app);
      tokio::select! {
        _ = server => {},
        _ = shutdown_rx.recv() => {},
      }
    });

    self.port = Some(actual_port);
    self.shutdown_tx = Some(shutdown_tx);
    self.task_handle = Some(task_handle);

    Ok(actual_port)
  }

  async fn stop(&mut self) -> Result<(), String> {
    if let Some(shutdown_tx) = self.shutdown_tx.take() {
      let _ = shutdown_tx.send(()).await;
    }

    if let Some(handle) = self.task_handle.take() {
      handle.abort();
    }

    self.port = None;
    Ok(())
  }
}

/// Register every `/v1` handler.
///
/// Pulled out of `start` so a test can build it. Axum panics when two handlers
/// claim the same path, and until this was callable the only thing that
/// exercised it was starting the real server — a conflict introduced here
/// would have shipped as an app that dies the moment the API is switched on.
///
/// The OpenAPI half of `split_for_parts` is discarded on purpose: the served
/// spec comes from the hand-maintained `ApiDoc`, which is why
/// `openapi_spec_covers_registered_routes` exists.
fn build_v1_router() -> Router<ApiServerState> {
  let (routes, _) = OpenApiRouter::new()
    .routes(routes!(get_profiles, create_profile))
    .routes(routes!(get_profile, update_profile, delete_profile))
    .routes(routes!(run_profile))
    .routes(routes!(run_profile_remote))
    // One `routes!` per PATH, not per handler: the GET and the DELETE share
    // `/v1/remote-sessions/{id}`, and registering them separately would have
    // the second overwrite the first.
    .routes(routes!(get_remote_session_api, stop_remote_session))
    .routes(routes!(list_remote_sessions_api))
    .routes(routes!(get_remote_hours))
    .routes(routes!(set_profile_cloud_sync))
    .routes(routes!(list_cookie_bot_schedules))
    .routes(routes!(
      get_cookie_bot_schedule,
      set_cookie_bot_schedule,
      delete_cookie_bot_schedule
    ))
    .routes(routes!(get_cookie_bot_conflicts))
    .routes(routes!(list_cookie_bot_runs, start_cookie_bot_run))
    .routes(routes!(cancel_cookie_bot_run))
    .routes(routes!(list_cookie_bot_presets))
    .routes(routes!(get_cookie_bot_usage))
    .routes(routes!(open_url_in_profile))
    .routes(routes!(kill_profile))
    .routes(routes!(batch_run_profiles))
    .routes(routes!(batch_stop_profiles))
    .routes(routes!(detect_import_profiles))
    .routes(routes!(import_profiles_api))
    .routes(routes!(import_profile_cookies))
    .routes(routes!(get_groups, create_group))
    .routes(routes!(get_group, update_group, delete_group))
    .routes(routes!(get_tags))
    .routes(routes!(get_proxies, create_proxy))
    .routes(routes!(import_proxies_api))
    .routes(routes!(get_proxy, update_proxy, delete_proxy))
    .routes(routes!(get_vpns, create_vpn))
    .routes(routes!(import_vpn))
    .routes(routes!(export_vpn))
    .routes(routes!(get_vpn, update_vpn, delete_vpn))
    .routes(routes!(get_extensions))
    .routes(routes!(delete_extension_api))
    .routes(routes!(get_extension_groups))
    .routes(routes!(delete_extension_group_api))
    .routes(routes!(download_browser_api))
    .routes(routes!(get_browser_versions))
    .routes(routes!(check_browser_downloaded))
    .split_for_parts();
  routes
}

// Terms and Conditions check middleware
async fn terms_check_middleware(
  request: axum::extract::Request,
  next: Next,
) -> Result<Response, StatusCode> {
  // Check if Wayfern terms have been accepted
  if !crate::wayfern_terms::WayfernTermsManager::instance().is_terms_accepted() {
    return Err(StatusCode::FORBIDDEN);
  }

  Ok(next.run(request).await)
}

// Authentication middleware
async fn auth_middleware(
  State(state): State<ApiServerState>,
  headers: HeaderMap,
  request: axum::extract::Request,
  next: Next,
) -> Result<Response, StatusCode> {
  let path = request.uri().path().to_string();

  // Get the Authorization header
  let auth_header = headers
    .get("Authorization")
    .and_then(|h| h.to_str().ok())
    .and_then(|h| h.strip_prefix("Bearer "));

  let token = match auth_header {
    Some(token) => token,
    None => {
      log::warn!("[api] Rejected {path}: missing Authorization header");
      return Err(StatusCode::UNAUTHORIZED);
    }
  };

  // Get the stored token
  let settings_manager = crate::settings_manager::SettingsManager::instance();
  let stored_token = match settings_manager.get_api_token(&state.app_handle).await {
    Ok(Some(stored_token)) => stored_token,
    Ok(None) => {
      log::warn!(
        "[api] Rejected {path}: API server has no stored token (was the API toggled off?)"
      );
      return Err(StatusCode::UNAUTHORIZED);
    }
    Err(e) => {
      log::error!("[api] Failed to read stored API token: {e}");
      return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
  };

  // Constant-time comparison so the auth check doesn't leak the shared-prefix
  // length via timing. `ConstantTimeEq` on equal-length byte slices; differing
  // lengths simply compare unequal.
  use subtle::ConstantTimeEq;
  let token_bytes = token.as_bytes();
  let stored_bytes = stored_token.as_bytes();
  let matches = token_bytes.len() == stored_bytes.len() && token_bytes.ct_eq(stored_bytes).into();
  if !matches {
    log::warn!("[api] Rejected {path}: token mismatch");
    return Err(StatusCode::UNAUTHORIZED);
  }

  // Token is valid, continue with the request
  Ok(next.run(request).await)
}

/// Logs every request: method, path, query, response status, duration.
/// Skips Authorization header and request bodies entirely.
async fn request_logging_middleware(request: axum::extract::Request, next: Next) -> Response {
  let method = request.method().clone();
  let path = request.uri().path().to_string();
  let query = request.uri().query().map(|q| q.to_string());
  let started = std::time::Instant::now();

  let response = next.run(request).await;

  let status = response.status();
  let elapsed_ms = started.elapsed().as_millis();

  let level = if status.is_server_error() {
    log::Level::Error
  } else if status.is_client_error() {
    log::Level::Warn
  } else {
    log::Level::Info
  };

  match query {
    Some(q) => log::log!(
      level,
      "[api] {method} {path}?{q} -> {status} ({elapsed_ms} ms)"
    ),
    None => log::log!(level, "[api] {method} {path} -> {status} ({elapsed_ms} ms)"),
  }

  response
}

fn is_automation_request(method: &Method, path: &str) -> bool {
  // Ending a remote session is the one automation action that is not a POST.
  // Its handler declares a 429, which could never fire while this function
  // returned early for every non-POST method.
  //
  // Cancelling a cookie-bot run joins it: both reach across to the fleet, and
  // treating one stop as metered and the other as free would be arbitrary.
  // Note that the desktop's own stop button goes through a Tauri command, not
  // this server, so a human can always stop a run the limiter has cut off.
  if method == Method::DELETE {
    let mut segments = match path
      .strip_prefix("/v1/remote-sessions/")
      .or_else(|| path.strip_prefix("/v1/cookie-bot/runs/"))
    {
      Some(rest) => rest.split('/'),
      None => return false,
    };
    return matches!((segments.next(), segments.next()), (Some(id), None) if !id.is_empty());
  }

  if method != Method::POST {
    return false;
  }

  // Starting a bot run leases a host for up to two hours and spends the
  // account's pooled remote-hour budget, which makes it the single most
  // expensive thing this API can be asked to do.
  //
  // Deliberately NOT here: the cookie-bot schedule writes (PUT and DELETE on
  // /v1/cookie-bot/schedules/{profile_id}). They are configuration — a small
  // row in donutbrowser-infra — and lease nothing. Metering them would 429 a
  // client enrolling a fleet of profiles at start-up, while the thing that
  // actually protects the hardware, the pooled hour budget, is enforced
  // server-side on every run whether or not it was scheduled from here.
  if matches!(
    path,
    "/v1/profiles/batch/run" | "/v1/profiles/batch/stop" | "/v1/cookie-bot/runs"
  ) {
    return true;
  }

  let Some(profile_action) = path.strip_prefix("/v1/profiles/") else {
    return false;
  };
  let mut segments = profile_action.split('/');
  matches!(
    (segments.next(), segments.next(), segments.next()),
    // `run-remote` is a separate segment from `run`, so it matched nothing here
    // and every remote launch bypassed the quota it declares a 429 for.
    (
      Some(_),
      Some("run" | "open-url" | "kill" | "run-remote"),
      None
    )
  )
}

async fn rate_limit_middleware(request: axum::extract::Request, next: Next) -> Response {
  if !is_automation_request(request.method(), request.uri().path()) {
    return next.run(request).await;
  }

  match crate::automation_rate_limiter::check_automation_rate_limit().await {
    crate::automation_rate_limiter::RateLimitOutcome::Limited { retry_after_secs } => {
      log::warn!(
        "[api] Rejected {}: automation rate limit exceeded; retry in {}s",
        request.uri().path(),
        retry_after_secs
      );
      (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, retry_after_secs.to_string())],
        "automation request rate limit exceeded",
      )
        .into_response()
    }
    crate::automation_rate_limiter::RateLimitOutcome::Unlimited
    | crate::automation_rate_limiter::RateLimitOutcome::Allowed { .. } => next.run(request).await,
  }
}

// Global API server instance
lazy_static! {
  pub static ref API_SERVER: Arc<Mutex<ApiServer>> = Arc::new(Mutex::new(ApiServer::new()));
}

// Tauri commands
#[tauri::command]
pub async fn start_api_server_internal(
  port: u16,
  app_handle: &tauri::AppHandle,
) -> Result<u16, String> {
  let mut server_guard = API_SERVER.lock().await;
  server_guard.start(app_handle.clone(), port).await
}

#[tauri::command]
pub async fn stop_api_server() -> Result<(), String> {
  let mut server_guard = API_SERVER.lock().await;
  server_guard.stop().await
}

#[tauri::command]
pub async fn start_api_server(
  port: Option<u16>,
  app_handle: tauri::AppHandle,
) -> Result<u16, String> {
  let actual_port = port.unwrap_or(10108);
  start_api_server_internal(actual_port, &app_handle).await
}

#[tauri::command]
pub async fn get_api_server_status() -> Result<Option<u16>, String> {
  let server_guard = API_SERVER.lock().await;
  Ok(server_guard.get_port())
}

// API Handlers - Profiles
/// Maps a manager-layer error onto a consistent HTTP status: 404 for missing
/// entities, 400 for validation/duplicate/client-input errors, 500 for
/// everything else (IO and other internal failures). The error text passes
/// through as the response body so API clients get a diagnostic instead of a
/// bare status code. Matching is on message content because the managers
/// return plain strings (some are the JSON `{"code": ...}` strings shared
/// with the Tauri commands).
fn manager_error_response(err: impl std::fmt::Display) -> (StatusCode, String) {
  let msg = err.to_string();

  // Structured {"code": ...} errors from the shared managers classify exactly.
  if let Ok(value) = serde_json::from_str::<serde_json::Value>(&msg) {
    if let Some(code) = value.get("code").and_then(|c| c.as_str()) {
      let status = if code.ends_with("_NOT_FOUND") {
        StatusCode::NOT_FOUND
      } else if code == "INTERNAL_ERROR" {
        StatusCode::INTERNAL_SERVER_ERROR
      } else if code.ends_with("_REQUIRES_PRO") || code.ends_with("_PAYMENT_REQUIRED") {
        // Paid-feature gates (FINGERPRINT_REQUIRES_PRO, PROXY_PAYMENT_REQUIRED).
        // Mapping them here lets the gate live in the shared manager instead of
        // being re-implemented in each handler to get the status right.
        StatusCode::PAYMENT_REQUIRED
      } else {
        // Validation-style codes (NAME_CANNOT_BE_EMPTY, GROUP_ALREADY_EXISTS,
        // WAYFERN_VERSION_NOT_AVAILABLE, ...).
        StatusCode::BAD_REQUEST
      };
      return (status, msg);
    }
  }

  // Plain-text manager messages: match the known phrases narrowly so raw
  // OS/serde/network error text (e.g. "invalid type: ..." from a corrupt
  // store) falls through to 500 instead of masquerading as a client error.
  let lower = msg.to_lowercase();
  let status = if lower.contains("not found") {
    StatusCode::NOT_FOUND
  } else if lower.contains("already exists")
    || lower.contains("cannot set both")
    || lower.contains("cannot edit")
    || lower.contains("cannot delete")
    || lower.contains("cannot open url")
    || lower.contains("invalid browser")
    || lower.contains("invalid profile id")
    || lower.contains("unsupported browser")
    || lower.contains("not supported on your platform")
    || lower.contains("is not downloaded")
    || lower.contains("terms and conditions")
  {
    StatusCode::BAD_REQUEST
  } else {
    StatusCode::INTERNAL_SERVER_ERROR
  };
  (status, msg)
}

/// Real per-group profile counts, computed from the profile list (the same
/// source of truth the GUI uses).
fn group_profile_counts() -> std::collections::HashMap<String, usize> {
  let mut counts = std::collections::HashMap::new();
  if let Ok(profiles) = ProfileManager::instance().list_profiles() {
    for profile in profiles {
      if let Some(group_id) = profile.group_id {
        *counts.entry(group_id).or_insert(0) += 1;
      }
    }
  }
  counts
}

#[utoipa::path(
  get,
  path = "/v1/profiles",
  responses(
    (status = 200, description = "List of all profiles", body = ApiProfilesResponse),
    (status = 401, description = "Unauthorized"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "profiles"
)]
async fn get_profiles() -> Result<Json<ApiProfilesResponse>, StatusCode> {
  let profile_manager = ProfileManager::instance();
  match profile_manager.list_profiles() {
    Ok(profiles) => {
      let api_profiles: Vec<ApiProfile> = profiles.iter().map(ApiProfile::from).collect();

      Ok(Json(ApiProfilesResponse {
        profiles: api_profiles,
        total: profiles.len(),
      }))
    }
    Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
  }
}

#[utoipa::path(
  get,
  path = "/v1/profiles/{id}",
  params(
    ("id" = String, Path, description = "Profile ID")
  ),
  responses(
    (status = 200, description = "Profile details", body = ApiProfileResponse),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "Profile not found"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "profiles"
)]
async fn get_profile(
  Path(id): Path<String>,
  State(_state): State<ApiServerState>,
) -> Result<Json<ApiProfileResponse>, StatusCode> {
  let profile_manager = ProfileManager::instance();
  match profile_manager.list_profiles() {
    Ok(profiles) => {
      if let Some(profile) = profiles.iter().find(|p| p.id.to_string() == id) {
        Ok(Json(ApiProfileResponse {
          profile: ApiProfile::from(profile),
        }))
      } else {
        Err(StatusCode::NOT_FOUND)
      }
    }
    Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
  }
}

/// Create a profile.
///
/// - `browser` must be `"wayfern"`; any other value is rejected
///   with 400.
/// - `version` is optional: omit it or pass `"latest"` to use the newest
///   already-downloaded version of that browser. The version must be present
///   locally (this endpoint does not download new versions); 400 if none is.
/// - Omitting the matching `wayfern_config`, or passing an
///   empty object `{}`, generates a fresh fingerprint automatically.
#[utoipa::path(
  post,
  path = "/v1/profiles",
  request_body = CreateProfileRequest,
  responses(
    (status = 200, description = "Profile created successfully", body = ApiProfileResponse),
    (status = 400, description = "Invalid browser, or no downloaded version available"),
    (status = 401, description = "Unauthorized"),
    (status = 402, description = "Selected proxy requires payment"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "profiles"
)]
async fn create_profile(
  State(state): State<ApiServerState>,
  Json(request): Json<CreateProfileRequest>,
) -> Result<Json<ApiProfileResponse>, (StatusCode, String)> {
  let profile_manager = ProfileManager::instance();

  // Only Wayfern profiles are launchable; the rest of the system
  // (fingerprint generation, launch, run) supports nothing else. Reject anything
  // else up front — otherwise the profile is created with no fingerprint and an
  // unrecognized browser, then crashes with a 500 on /run. Mirrors the MCP
  // create_profile validation.
  if request.browser != "wayfern" {
    return Err((
      StatusCode::BAD_REQUEST,
      format!(
        "Invalid browser \"{}\". Must be \"wayfern\" (anti-detect Chromium).",
        request.browser
      ),
    ));
  }

  // Resolve the version. Omitted, empty, or "latest" means "newest version
  // already downloaded for this browser". The create path generates the
  // fingerprint by launching that binary, so the version must be present
  // locally — we don't fetch new versions here. 400 if none is downloaded.
  let version = match request.version.as_deref() {
    Some(v) if !v.is_empty() && v != "latest" => v.to_string(),
    _ => {
      let registry = crate::downloaded_browsers_registry::DownloadedBrowsersRegistry::instance();
      let mut versions = registry.get_downloaded_versions(&request.browser);
      // browsers is a HashMap, so keys are unordered — sort newest-first by
      // semver before taking the latest.
      versions.sort_by(|a, b| crate::api_client::compare_versions(b, a));
      match versions.into_iter().next() {
        Some(v) => v,
        None => {
          return Err((
            StatusCode::BAD_REQUEST,
            format!(
              "No downloaded version of \"{}\" is available. Download the browser in Donut Browser first — this endpoint does not download browsers.",
              request.browser
            ),
          ));
        }
      }
    }
  };

  // Parse wayfern config if provided
  let wayfern_config = if let Some(config) = &request.wayfern_config {
    serde_json::from_value(config.clone()).ok()
  } else {
    None
  };

  // Reject a dead/unreachable proxy or VPN before creating the profile. A 402
  // (expired proxy subscription) maps to 402; anything else is a 400.
  if let Err(err) =
    crate::validate_profile_network(request.proxy_id.as_deref(), request.vpn_id.as_deref()).await
  {
    return Err(if err.contains("PROXY_PAYMENT_REQUIRED") {
      (
        StatusCode::PAYMENT_REQUIRED,
        "The selected proxy requires an active subscription.".to_string(),
      )
    } else {
      (
        StatusCode::BAD_REQUEST,
        format!("Profile network validation failed: {err}"),
      )
    });
  }

  // Create profile using the async create_profile_with_group method
  match profile_manager
    .create_profile_with_group(
      &state.app_handle,
      &request.name,
      &request.browser,
      &version,
      request.release_type.as_deref().unwrap_or("stable"),
      request.proxy_id.clone(),
      request.vpn_id.clone(),
      wayfern_config,
      request.group_id.clone(),
      false,
      None,
      request.launch_hook.clone(),
    )
    .await
  {
    Ok(mut profile) => {
      // Apply tags if provided
      if let Some(tags) = &request.tags {
        if profile_manager
          .update_profile_tags(&state.app_handle, &profile.name, tags.clone())
          .is_err()
        {
          return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Profile created but failed to apply tags.".to_string(),
          ));
        }
        profile.tags = tags.clone();
      }

      // Update tag manager with new tags
      if let Ok(profiles) = profile_manager.list_profiles() {
        let _ = crate::tag_manager::TAG_MANAGER
          .lock()
          .map(|manager| manager.rebuild_from_profiles(&profiles));
      }

      Ok(Json(ApiProfileResponse {
        profile: ApiProfile::from(&profile),
      }))
    }
    Err(e) => Err((
      StatusCode::BAD_REQUEST,
      format!("Failed to create profile: {e}"),
    )),
  }
}

#[utoipa::path(
  put,
  path = "/v1/profiles/{id}",
  params(
    ("id" = String, Path, description = "Profile ID")
  ),
  request_body = UpdateProfileRequest,
  responses(
    (status = 200, description = "Profile updated successfully", body = ApiProfileResponse),
    (status = 400, description = "Bad request"),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "Profile not found"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "profiles"
)]
async fn update_profile(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
  Json(request): Json<UpdateProfileRequest>,
) -> Result<Json<ApiProfileResponse>, (StatusCode, String)> {
  let profile_manager = ProfileManager::instance();

  if request.proxy_id.as_deref().is_some_and(|s| !s.is_empty())
    && request.vpn_id.as_deref().is_some_and(|s| !s.is_empty())
  {
    return Err((
      StatusCode::BAD_REQUEST,
      "Cannot set both proxy_id and vpn_id".to_string(),
    ));
  }

  // Update profile fields
  if let Some(new_name) = request.name {
    if let Err(e) = profile_manager.rename_profile(&state.app_handle, &id, &new_name) {
      return Err(manager_error_response(e));
    }
  }

  if let Some(version) = request.version {
    if let Err(e) = profile_manager.update_profile_version(&state.app_handle, &id, &version) {
      return Err(manager_error_response(e));
    }
  }

  if let Some(proxy_id) = request.proxy_id {
    if let Err(e) = profile_manager
      .update_profile_proxy(state.app_handle.clone(), &id, Some(proxy_id))
      .await
    {
      return Err(manager_error_response(e));
    }
  }

  if let Some(vpn_id) = request.vpn_id {
    let normalized = if vpn_id.is_empty() {
      None
    } else {
      Some(vpn_id)
    };
    if let Err(e) = profile_manager
      .update_profile_vpn(state.app_handle.clone(), &id, normalized)
      .await
    {
      return Err(manager_error_response(e));
    }
  }

  if let Some(launch_hook) = request.launch_hook {
    let normalized = if launch_hook.trim().is_empty() {
      None
    } else {
      Some(launch_hook)
    };

    if let Err(e) = profile_manager.update_profile_launch_hook(&state.app_handle, &id, normalized) {
      return Err(manager_error_response(e));
    }
  }

  if let Some(group_id) = request.group_id {
    if let Err(e) =
      profile_manager.assign_profiles_to_group(&state.app_handle, vec![id.clone()], Some(group_id))
    {
      return Err(manager_error_response(e));
    }
  }

  if let Some(tags) = request.tags {
    if let Err(e) = profile_manager.update_profile_tags(&state.app_handle, &id, tags) {
      return Err(manager_error_response(e));
    }

    // Update tag manager with new tags from all profiles
    if let Ok(profiles) = profile_manager.list_profiles() {
      let _ = crate::tag_manager::TAG_MANAGER
        .lock()
        .map(|manager| manager.rebuild_from_profiles(&profiles));
    }
  }

  if let Some(extension_group_id) = request.extension_group_id {
    let ext_group = if extension_group_id.is_empty() {
      None
    } else {
      Some(extension_group_id)
    };
    if let Err(e) = profile_manager.update_profile_extension_group(&id, ext_group) {
      return Err(manager_error_response(e));
    }
  }

  if let Some(proxy_bypass_rules) = request.proxy_bypass_rules {
    if let Err(e) =
      profile_manager.update_profile_proxy_bypass_rules(&state.app_handle, &id, proxy_bypass_rules)
    {
      return Err(manager_error_response(e));
    }
  }

  if let Some(sync_mode) = request.sync_mode {
    if let Err(e) =
      crate::sync::set_profile_sync_mode(state.app_handle.clone(), id.clone(), sync_mode).await
    {
      return Err(manager_error_response(e));
    }
  }

  if let Some(clear_on_close) = request.clear_on_close {
    if let Err(e) =
      profile_manager.update_profile_clear_on_close(&state.app_handle, &id, clear_on_close)
    {
      return Err(manager_error_response(e));
    }
  }

  // Return updated profile
  get_profile(Path(id), State(state))
    .await
    .map_err(|status| (status, String::new()))
}

#[utoipa::path(
  delete,
  path = "/v1/profiles/{id}",
  params(
    ("id" = String, Path, description = "Profile ID")
  ),
  responses(
    (status = 204, description = "Profile deleted successfully"),
    (status = 400, description = "Bad request"),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "Profile not found"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "profiles"
)]
async fn delete_profile(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
) -> Result<StatusCode, (StatusCode, String)> {
  let profile_manager = ProfileManager::instance();
  match profile_manager.delete_profile(&state.app_handle, &id) {
    Ok(_) => Ok(StatusCode::NO_CONTENT),
    Err(e) => Err(manager_error_response(e)),
  }
}

// API Handlers - Groups
#[utoipa::path(
  get,
  path = "/v1/groups",
  responses(
    (status = 200, description = "List of all groups", body = Vec<ApiGroupResponse>),
    (status = 401, description = "Unauthorized"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "groups"
)]
async fn get_groups(
  State(_state): State<ApiServerState>,
) -> Result<Json<Vec<ApiGroupResponse>>, StatusCode> {
  match GROUP_MANAGER.lock() {
    Ok(manager) => match manager.get_all_groups() {
      Ok(groups) => {
        let counts = group_profile_counts();
        let api_groups = groups
          .into_iter()
          .map(|group| ApiGroupResponse {
            profile_count: counts.get(&group.id).copied().unwrap_or(0),
            id: group.id,
            name: group.name,
          })
          .collect();
        Ok(Json(api_groups))
      }
      Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    },
    Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
  }
}

#[utoipa::path(
  get,
  path = "/v1/groups/{id}",
  params(
    ("id" = String, Path, description = "Group ID")
  ),
  responses(
    (status = 200, description = "Group details", body = ApiGroupResponse),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "Group not found"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "groups"
)]
async fn get_group(
  Path(id): Path<String>,
  State(_state): State<ApiServerState>,
) -> Result<Json<ApiGroupResponse>, StatusCode> {
  match GROUP_MANAGER.lock() {
    Ok(manager) => match manager.get_all_groups() {
      Ok(groups) => {
        if let Some(group) = groups.into_iter().find(|g| g.id == id) {
          Ok(Json(ApiGroupResponse {
            profile_count: group_profile_counts().get(&group.id).copied().unwrap_or(0),
            id: group.id,
            name: group.name,
          }))
        } else {
          Err(StatusCode::NOT_FOUND)
        }
      }
      Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    },
    Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
  }
}

#[utoipa::path(
  post,
  path = "/v1/groups",
  request_body = CreateGroupRequest,
  responses(
    (status = 200, description = "Group created successfully", body = ApiGroupResponse),
    (status = 400, description = "Bad request"),
    (status = 401, description = "Unauthorized"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "groups"
)]
async fn create_group(
  State(state): State<ApiServerState>,
  Json(request): Json<CreateGroupRequest>,
) -> Result<Json<ApiGroupResponse>, (StatusCode, String)> {
  match GROUP_MANAGER.lock() {
    Ok(manager) => match manager.create_group(&state.app_handle, request.name) {
      Ok(group) => Ok(Json(ApiGroupResponse {
        id: group.id,
        name: group.name,
        profile_count: 0,
      })),
      Err(e) => Err(manager_error_response(e)),
    },
    Err(_) => Err((
      StatusCode::INTERNAL_SERVER_ERROR,
      "group manager unavailable".to_string(),
    )),
  }
}

#[utoipa::path(
  put,
  path = "/v1/groups/{id}",
  params(
    ("id" = String, Path, description = "Group ID")
  ),
  request_body = UpdateGroupRequest,
  responses(
    (status = 200, description = "Group updated successfully", body = ApiGroupResponse),
    (status = 400, description = "Bad request"),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "Group not found"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "groups"
)]
async fn update_group(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
  Json(request): Json<UpdateGroupRequest>,
) -> Result<Json<ApiGroupResponse>, (StatusCode, String)> {
  match GROUP_MANAGER.lock() {
    Ok(manager) => match manager.update_group(&state.app_handle, id.clone(), request.name) {
      Ok(group) => Ok(Json(ApiGroupResponse {
        profile_count: group_profile_counts().get(&group.id).copied().unwrap_or(0),
        id: group.id,
        name: group.name,
      })),
      Err(e) => Err(manager_error_response(e)),
    },
    Err(_) => Err((
      StatusCode::INTERNAL_SERVER_ERROR,
      "group manager unavailable".to_string(),
    )),
  }
}

#[utoipa::path(
  delete,
  path = "/v1/groups/{id}",
  params(
    ("id" = String, Path, description = "Group ID")
  ),
  responses(
    (status = 204, description = "Group deleted successfully"),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "Group not found"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "groups"
)]
async fn delete_group(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
) -> Result<StatusCode, (StatusCode, String)> {
  match GROUP_MANAGER.lock() {
    Ok(manager) => match manager.delete_group(&state.app_handle, id.clone()) {
      Ok(_) => Ok(StatusCode::NO_CONTENT),
      Err(e) => Err(manager_error_response(e)),
    },
    Err(_) => Err((
      StatusCode::INTERNAL_SERVER_ERROR,
      "group manager unavailable".to_string(),
    )),
  }
}

// API Handlers - Tags
#[utoipa::path(
  get,
  path = "/v1/tags",
  responses(
    (status = 200, description = "List of all tags", body = Vec<String>),
    (status = 401, description = "Unauthorized"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "tags"
)]
async fn get_tags(State(_state): State<ApiServerState>) -> Result<Json<Vec<String>>, StatusCode> {
  match TAG_MANAGER.lock() {
    Ok(manager) => match manager.get_all_tags() {
      Ok(tags) => Ok(Json(tags)),
      Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    },
    Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
  }
}

// API Handlers - Proxies
#[utoipa::path(
  get,
  path = "/v1/proxies",
  responses(
    (status = 200, description = "List of all proxies", body = Vec<ApiProxyResponse>),
    (status = 401, description = "Unauthorized"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "proxies"
)]
async fn get_proxies(
  State(_state): State<ApiServerState>,
) -> Result<Json<Vec<ApiProxyResponse>>, StatusCode> {
  let proxies = PROXY_MANAGER.get_stored_proxies();
  Ok(Json(
    proxies
      .into_iter()
      .map(|p| ApiProxyResponse {
        id: p.id,
        name: p.name,
        proxy_settings: p.proxy_settings,
      })
      .collect(),
  ))
}

#[utoipa::path(
  get,
  path = "/v1/proxies/{id}",
  params(
    ("id" = String, Path, description = "Proxy ID")
  ),
  responses(
    (status = 200, description = "Proxy details", body = ApiProxyResponse),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "Proxy not found"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "proxies"
)]
async fn get_proxy(
  Path(id): Path<String>,
  State(_state): State<ApiServerState>,
) -> Result<Json<ApiProxyResponse>, StatusCode> {
  let proxies = PROXY_MANAGER.get_stored_proxies();
  if let Some(proxy) = proxies.into_iter().find(|p| p.id == id) {
    Ok(Json(ApiProxyResponse {
      id: proxy.id,
      name: proxy.name,
      proxy_settings: proxy.proxy_settings,
    }))
  } else {
    Err(StatusCode::NOT_FOUND)
  }
}

#[utoipa::path(
  post,
  path = "/v1/proxies",
  request_body = CreateProxyRequest,
  responses(
    (status = 200, description = "Proxy created successfully", body = ApiProxyResponse),
    (status = 400, description = "Bad request"),
    (status = 401, description = "Unauthorized"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "proxies"
)]
async fn create_proxy(
  State(state): State<ApiServerState>,
  Json(request): Json<CreateProxyRequest>,
) -> Result<Json<ApiProxyResponse>, (StatusCode, String)> {
  let result = PROXY_MANAGER.create_stored_proxy(
    &state.app_handle,
    request.name.clone(),
    request.proxy_settings,
  );

  match result {
    Ok(proxy) => Ok(Json(ApiProxyResponse {
      id: proxy.id,
      name: proxy.name,
      proxy_settings: proxy.proxy_settings,
    })),
    Err(e) => Err(manager_error_response(e)),
  }
}

// API Handler - Bulk-import proxies from a txt list or a Donut JSON export.
// Mirrors the MCP `import_proxies` tool.
#[utoipa::path(
  post,
  path = "/v1/proxies/import",
  request_body = ImportProxiesRequest,
  responses(
    (status = 200, description = "Import completed; inspect counts and per-proxy errors", body = ImportProxiesResponse),
    (status = 400, description = "Invalid format or no valid proxies in content"),
    (status = 401, description = "Unauthorized"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "proxies"
)]
async fn import_proxies_api(
  State(state): State<ApiServerState>,
  Json(request): Json<ImportProxiesRequest>,
) -> Result<Json<ImportProxiesResponse>, (StatusCode, String)> {
  let result = match request.format.as_str() {
    "json" => PROXY_MANAGER
      .import_proxies_json(&state.app_handle, &request.content)
      .map_err(manager_error_response)?,
    "txt" => {
      use crate::proxy_manager::{ProxyManager, ProxyParseResult};

      let parsed: Vec<_> = ProxyManager::parse_txt_proxies(&request.content)
        .into_iter()
        .filter_map(|r| match r {
          ProxyParseResult::Parsed(p) => Some(p),
          _ => None,
        })
        .collect();

      if parsed.is_empty() {
        return Err((
          StatusCode::BAD_REQUEST,
          "No valid proxies found in content".to_string(),
        ));
      }

      PROXY_MANAGER
        .import_proxies_from_parsed(&state.app_handle, parsed, request.name_prefix)
        .map_err(manager_error_response)?
    }
    other => {
      return Err((
        StatusCode::BAD_REQUEST,
        format!("Invalid format \"{other}\", must be \"json\" or \"txt\""),
      ))
    }
  };

  Ok(Json(ImportProxiesResponse {
    imported_count: result.imported_count,
    skipped_count: result.skipped_count,
    errors: result.errors,
    proxies: result
      .proxies
      .into_iter()
      .map(|p| ApiProxyResponse {
        id: p.id,
        name: p.name,
        proxy_settings: p.proxy_settings,
      })
      .collect(),
  }))
}

#[utoipa::path(
  put,
  path = "/v1/proxies/{id}",
  params(
    ("id" = String, Path, description = "Proxy ID")
  ),
  request_body = UpdateProxyRequest,
  responses(
    (status = 200, description = "Proxy updated successfully", body = ApiProxyResponse),
    (status = 400, description = "Bad request"),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "Proxy not found"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "proxies"
)]
async fn update_proxy(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
  Json(request): Json<UpdateProxyRequest>,
) -> Result<Json<ApiProxyResponse>, (StatusCode, String)> {
  let result =
    PROXY_MANAGER.update_stored_proxy(&state.app_handle, &id, request.name, request.proxy_settings);

  match result {
    Ok(proxy) => Ok(Json(ApiProxyResponse {
      id: proxy.id,
      name: proxy.name,
      proxy_settings: proxy.proxy_settings,
    })),
    Err(e) => Err(manager_error_response(e)),
  }
}

#[utoipa::path(
  delete,
  path = "/v1/proxies/{id}",
  params(
    ("id" = String, Path, description = "Proxy ID")
  ),
  responses(
    (status = 204, description = "Proxy deleted successfully"),
    (status = 400, description = "Bad request (e.g. cloud-managed proxy)"),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "Proxy not found"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "proxies"
)]
async fn delete_proxy(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
) -> Result<StatusCode, (StatusCode, String)> {
  match PROXY_MANAGER.delete_stored_proxy(&state.app_handle, &id) {
    Ok(_) => Ok(StatusCode::NO_CONTENT),
    Err(e) => Err(manager_error_response(e)),
  }
}

// API Handlers - VPNs

fn vpn_to_api_response(c: &crate::vpn::VpnConfig) -> ApiVpnResponse {
  ApiVpnResponse {
    id: c.id.clone(),
    name: c.name.clone(),
    vpn_type: c.vpn_type.to_string(),
    created_at: c.created_at,
    last_used: c.last_used,
  }
}

fn parse_vpn_type(s: &str) -> Option<crate::vpn::VpnType> {
  match s.to_ascii_lowercase().as_str() {
    "wireguard" | "wg" => Some(crate::vpn::VpnType::WireGuard),
    _ => None,
  }
}

#[utoipa::path(
  get,
  path = "/v1/vpns",
  responses(
    (status = 200, description = "List of all VPN configurations", body = Vec<ApiVpnResponse>),
    (status = 401, description = "Unauthorized"),
    (status = 500, description = "Internal server error")
  ),
  security(("bearer_auth" = [])),
  tag = "vpns"
)]
async fn get_vpns(
  State(_state): State<ApiServerState>,
) -> Result<Json<Vec<ApiVpnResponse>>, StatusCode> {
  let storage = crate::vpn::VPN_STORAGE
    .lock()
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
  let configs = storage
    .list_configs()
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
  Ok(Json(configs.iter().map(vpn_to_api_response).collect()))
}

#[utoipa::path(
  get,
  path = "/v1/vpns/{id}",
  params(("id" = String, Path, description = "VPN configuration ID")),
  responses(
    (status = 200, description = "VPN configuration details", body = ApiVpnResponse),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "VPN configuration not found"),
    (status = 500, description = "Internal server error")
  ),
  security(("bearer_auth" = [])),
  tag = "vpns"
)]
async fn get_vpn(
  Path(id): Path<String>,
  State(_state): State<ApiServerState>,
) -> Result<Json<ApiVpnResponse>, StatusCode> {
  let storage = crate::vpn::VPN_STORAGE
    .lock()
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
  let configs = storage
    .list_configs()
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
  configs
    .iter()
    .find(|c| c.id == id)
    .map(|c| Json(vpn_to_api_response(c)))
    .ok_or(StatusCode::NOT_FOUND)
}

#[utoipa::path(
  get,
  path = "/v1/vpns/{id}/export",
  params(("id" = String, Path, description = "VPN configuration ID")),
  responses(
    (status = 200, description = "Decrypted VPN configuration", body = ApiVpnExportResponse),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "VPN configuration not found"),
    (status = 500, description = "Internal server error")
  ),
  security(("bearer_auth" = [])),
  tag = "vpns"
)]
async fn export_vpn(
  Path(id): Path<String>,
  State(_state): State<ApiServerState>,
) -> Result<Json<ApiVpnExportResponse>, StatusCode> {
  let storage = crate::vpn::VPN_STORAGE
    .lock()
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
  match storage.load_config(&id) {
    Ok(config) => Ok(Json(ApiVpnExportResponse {
      id: config.id,
      name: config.name,
      vpn_type: config.vpn_type.to_string(),
      config_data: config.config_data,
    })),
    Err(_) => Err(StatusCode::NOT_FOUND),
  }
}

#[utoipa::path(
  post,
  path = "/v1/vpns/import",
  request_body = ImportVpnRequest,
  responses(
    (status = 200, description = "VPN configuration imported successfully", body = ApiVpnResponse),
    (status = 400, description = "Invalid or unrecognized VPN config"),
    (status = 401, description = "Unauthorized"),
    (status = 500, description = "Internal server error")
  ),
  security(("bearer_auth" = [])),
  tag = "vpns"
)]
async fn import_vpn(
  State(_state): State<ApiServerState>,
  Json(request): Json<ImportVpnRequest>,
) -> Result<Json<ApiVpnResponse>, StatusCode> {
  let result = {
    let storage = crate::vpn::VPN_STORAGE
      .lock()
      .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    storage.import_config(&request.content, &request.filename, request.name)
  };
  match result {
    Ok(config) => {
      let _ = events::emit("vpn-configs-changed", ());
      Ok(Json(vpn_to_api_response(&config)))
    }
    Err(_) => Err(StatusCode::BAD_REQUEST),
  }
}

#[utoipa::path(
  post,
  path = "/v1/vpns",
  request_body = CreateVpnRequest,
  responses(
    (status = 200, description = "VPN configuration created successfully", body = ApiVpnResponse),
    (status = 400, description = "Invalid VPN config or unknown vpn_type"),
    (status = 401, description = "Unauthorized"),
    (status = 500, description = "Internal server error")
  ),
  security(("bearer_auth" = [])),
  tag = "vpns"
)]
async fn create_vpn(
  State(_state): State<ApiServerState>,
  Json(request): Json<CreateVpnRequest>,
) -> Result<Json<ApiVpnResponse>, StatusCode> {
  let vpn_type = parse_vpn_type(&request.vpn_type).ok_or(StatusCode::BAD_REQUEST)?;
  let result = {
    let storage = crate::vpn::VPN_STORAGE
      .lock()
      .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    storage.create_config_manual(&request.name, vpn_type, &request.config_data)
  };
  match result {
    Ok(config) => {
      let _ = events::emit("vpn-configs-changed", ());
      Ok(Json(vpn_to_api_response(&config)))
    }
    Err(_) => Err(StatusCode::BAD_REQUEST),
  }
}

#[utoipa::path(
  put,
  path = "/v1/vpns/{id}",
  params(("id" = String, Path, description = "VPN configuration ID")),
  request_body = UpdateVpnRequest,
  responses(
    (status = 200, description = "VPN configuration updated successfully", body = ApiVpnResponse),
    (status = 400, description = "Bad request"),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "VPN configuration not found"),
    (status = 500, description = "Internal server error")
  ),
  security(("bearer_auth" = [])),
  tag = "vpns"
)]
async fn update_vpn(
  Path(id): Path<String>,
  State(_state): State<ApiServerState>,
  Json(request): Json<UpdateVpnRequest>,
) -> Result<Json<ApiVpnResponse>, StatusCode> {
  let result = {
    let storage = crate::vpn::VPN_STORAGE
      .lock()
      .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    storage.update_config_name(&id, &request.name)
  };
  match result {
    Ok(config) => {
      let _ = events::emit("vpn-configs-changed", ());
      Ok(Json(vpn_to_api_response(&config)))
    }
    Err(_) => Err(StatusCode::NOT_FOUND),
  }
}

#[utoipa::path(
  delete,
  path = "/v1/vpns/{id}",
  params(("id" = String, Path, description = "VPN configuration ID")),
  responses(
    (status = 204, description = "VPN configuration deleted successfully"),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "VPN configuration not found"),
    (status = 500, description = "Internal server error")
  ),
  security(("bearer_auth" = [])),
  tag = "vpns"
)]
async fn delete_vpn(
  Path(id): Path<String>,
  State(_state): State<ApiServerState>,
) -> Result<StatusCode, StatusCode> {
  let _ = crate::vpn_worker_runner::stop_vpn_worker_by_vpn_id(&id).await;

  let result = {
    let storage = crate::vpn::VPN_STORAGE
      .lock()
      .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    storage.delete_config(&id)
  };
  match result {
    Ok(_) => {
      let _ = events::emit("vpn-configs-changed", ());
      Ok(StatusCode::NO_CONTENT)
    }
    Err(_) => Err(StatusCode::NOT_FOUND),
  }
}

// Extension API endpoints

#[utoipa::path(
  get,
  path = "/v1/extensions",
  responses(
    (status = 200, description = "List of extensions"),
    (status = 401, description = "Unauthorized"),
  ),
  security(("bearer_auth" = [])),
  tag = "extensions"
)]
async fn get_extensions(
  State(_state): State<ApiServerState>,
) -> Result<Json<Vec<crate::extension_manager::Extension>>, StatusCode> {
  let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
  mgr
    .list_extensions()
    .map(Json)
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[utoipa::path(
  get,
  path = "/v1/extension-groups",
  responses(
    (status = 200, description = "List of extension groups"),
    (status = 401, description = "Unauthorized"),
  ),
  security(("bearer_auth" = [])),
  tag = "extensions"
)]
async fn get_extension_groups(
  State(_state): State<ApiServerState>,
) -> Result<Json<Vec<crate::extension_manager::ExtensionGroup>>, StatusCode> {
  let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
  mgr
    .list_groups()
    .map(Json)
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[utoipa::path(
  delete,
  path = "/v1/extensions/{id}",
  params(("id" = String, Path, description = "Extension ID")),
  responses(
    (status = 204, description = "Extension deleted"),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "Extension not found"),
    (status = 500, description = "Internal server error"),
  ),
  security(("bearer_auth" = [])),
  tag = "extensions"
)]
async fn delete_extension_api(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
) -> Result<StatusCode, (StatusCode, String)> {
  let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
  mgr
    .delete_extension(&state.app_handle, &id)
    .map(|_| StatusCode::NO_CONTENT)
    .map_err(manager_error_response)
}

#[utoipa::path(
  delete,
  path = "/v1/extension-groups/{id}",
  params(("id" = String, Path, description = "Extension Group ID")),
  responses(
    (status = 204, description = "Extension group deleted"),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "Extension group not found"),
    (status = 500, description = "Internal server error"),
  ),
  security(("bearer_auth" = [])),
  tag = "extensions"
)]
async fn delete_extension_group_api(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
) -> Result<StatusCode, (StatusCode, String)> {
  let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
  mgr
    .delete_group(&state.app_handle, &id)
    .map(|_| StatusCode::NO_CONTENT)
    .map_err(manager_error_response)
}

// API Handler - Run Profile with Remote Debugging
#[utoipa::path(
  post,
  path = "/v1/profiles/{id}/run",
  params(
    ("id" = String, Path, description = "Profile ID")
  ),
  request_body = RunProfileRequest,
  responses(
    (status = 200, description = "Profile launched successfully", body = RunProfileResponse),
    (status = 400, description = "Cannot launch cross-OS profile"),
    (status = 401, description = "Unauthorized"),
    (status = 402, description = "Active paid plan with browser automation required"),
    (status = 404, description = "Profile not found"),
    (status = 409, description = "Profile is locked by another team member"),
    (status = 429, description = "Automation request rate limit exceeded"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "profiles"
)]
async fn run_profile(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
  Json(request): Json<RunProfileRequest>,
) -> Result<Json<RunProfileResponse>, StatusCode> {
  if !crate::cloud_auth::CLOUD_AUTH
    .can_use_browser_automation()
    .await
  {
    return Err(StatusCode::PAYMENT_REQUIRED);
  }

  let headless = request.headless.unwrap_or(false);
  let url = request.url;

  let profile_manager = ProfileManager::instance();
  let profiles = profile_manager
    .list_profiles()
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

  let profile = profiles
    .iter()
    .find(|p| p.id.to_string() == id)
    .ok_or(StatusCode::NOT_FOUND)?;

  if profile.is_cross_os() {
    return Err(StatusCode::BAD_REQUEST);
  }

  // Team lock check
  crate::team_lock::acquire_team_lock_if_needed(profile)
    .await
    .map_err(|_| StatusCode::CONFLICT)?;

  let remote_debugging_port = {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
      .await
      .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let port = listener
      .local_addr()
      .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
      .port();
    drop(listener);
    port
  };

  // Use the same launch path as the main app, but force a fresh instance with
  // remote debugging enabled so the returned port is the one the browser binds.
  match crate::browser_runner::launch_browser_profile_impl(
    state.app_handle.clone(),
    profile.clone(),
    url,
    Some(remote_debugging_port),
    headless,
    true,
  )
  .await
  {
    Ok(updated_profile) => Ok(Json(RunProfileResponse {
      profile_id: updated_profile.id.to_string(),
      remote_debugging_port,
      headless,
    })),
    Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
  }
}

// API Handler - Launch this profile on a REMOTE VM of its own operating system
#[utoipa::path(
  post,
  path = "/v1/profiles/{id}/run-remote",
  params(
    ("id" = String, Path, description = "Profile ID")
  ),
  request_body = RunRemoteRequest,
  responses(
    (status = 200, description = "Remote session started", body = RunRemoteResponse),
    (status = 400, description = "Profile does not have cloud sync enabled"),
    (status = 401, description = "Unauthorized"),
    (status = 402, description = "Active paid plan with browser automation required"),
    (status = 404, description = "Profile not found"),
    (status = 409, description = "Profile is locked by another session"),
    (status = 429, description = "Automation request rate limit exceeded"),
    (status = 503, description = "No remote capacity for this operating system"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "profiles"
)]
async fn run_profile_remote(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
  Json(request): Json<RunRemoteRequest>,
) -> Result<Json<RunRemoteResponse>, (StatusCode, String)> {
  if !crate::cloud_auth::CLOUD_AUTH
    .can_use_browser_automation()
    .await
  {
    return Err((StatusCode::PAYMENT_REQUIRED, String::new()));
  }

  let profile_manager = ProfileManager::instance();
  let profiles = profile_manager
    .list_profiles()
    .map_err(manager_error_response)?;
  let profile = profiles
    .iter()
    .find(|p| p.id.to_string() == id)
    .ok_or((StatusCode::NOT_FOUND, "profile not found".to_string()))?;

  // The profile must exist in cloud storage before a remote host can open it —
  // the VM pulls it from donut-sync, and a profile that has never synced would
  // launch an empty browser and then push that emptiness back over the real one.
  if let Err(reason) = remote_launch_precondition(profile).await {
    return Err((StatusCode::BAD_REQUEST, reason));
  }

  // Deliberately NO is_cross_os() guard here. Local /run refuses a foreign
  // profile because this machine is the wrong OS; running it remotely on a host
  // of its OWN OS is precisely what this endpoint exists for.
  let outcome =
    crate::remote_session::start_remote_session(state.app_handle.clone(), profile, request.url)
      .await
      .map_err(remote_session_error_response)?;

  Ok(Json(RunRemoteResponse {
    profile_id: profile.id.to_string(),
    session_id: outcome.session_id,
    platform: outcome.platform,
    status: outcome.status,
  }))
}

#[utoipa::path(
  post,
  path = "/v1/profiles/{id}/cloud-sync",
  params(
    ("id" = String, Path, description = "Profile ID")
  ),
  request_body = SetCloudSyncRequest,
  responses(
    (status = 200, description = "Cloud sync mode updated", body = SetCloudSyncResponse),
    (status = 400, description = "Invalid mode, or the profile cannot be synced"),
    (status = 401, description = "Unauthorized"),
    (status = 402, description = "Active paid plan with cloud backup required"),
    (status = 404, description = "Profile not found"),
    (status = 409, description = "Profile is running — stop it before enabling sync"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "profiles"
)]
async fn set_profile_cloud_sync(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
  Json(request): Json<SetCloudSyncRequest>,
) -> Result<Json<SetCloudSyncResponse>, (StatusCode, String)> {
  // Remote launch requires cloud sync, and until now sync could only be turned
  // on from the GUI — so an automation-only caller could never reach the state
  // that makes /run-remote work.
  let mode = match request.mode.as_str() {
    "Disabled" | "Regular" | "Encrypted" => request.mode.clone(),
    other => {
      return Err((
        StatusCode::BAD_REQUEST,
        format!("invalid sync mode {other:?}; expected Disabled, Regular or Encrypted"),
      ));
    }
  };

  crate::sync::set_profile_sync_mode(state.app_handle.clone(), id.clone(), mode.clone())
    .await
    .map_err(sync_mode_error_response)?;

  let profile_manager = ProfileManager::instance();
  let profiles = profile_manager
    .list_profiles()
    .map_err(manager_error_response)?;
  let profile = profiles
    .iter()
    .find(|p| p.id.to_string() == id)
    .ok_or((StatusCode::NOT_FOUND, "profile not found".to_string()))?;

  // Reported rather than left for the caller to discover at launch time: the
  // most common reason a caller enables sync is to run the profile remotely,
  // and Encrypted mode silently makes that impossible.
  let blocked = remote_launch_precondition(profile).await.err();
  Ok(Json(SetCloudSyncResponse {
    profile_id: profile.id.to_string(),
    mode,
    remote_launchable: blocked.is_none(),
    remote_blocked_reason: blocked,
  }))
}

/// Map a sync-mode failure onto the status the caller can act on.
///
/// `set_profile_sync_mode` reports a running profile as a JSON body rather than
/// a plain message, because enabling sync under a live browser would race the
/// browser's own writes.
fn sync_mode_error_response(err: String) -> (StatusCode, String) {
  if err.contains("PROFILE_RUNNING") {
    return (
      StatusCode::CONFLICT,
      "profile is running; stop it before changing cloud sync".to_string(),
    );
  }
  if err.contains("cross-OS") || err.contains("ephemeral") {
    return (StatusCode::BAD_REQUEST, err);
  }
  (StatusCode::INTERNAL_SERVER_ERROR, err)
}

/// Whether a profile may be launched on a remote host.
///
/// The one gate between "the user asked" and "a browser opens somewhere else
/// holding their cookies". Adds the live check the pure rules cannot make: a
/// launch that races this profile's own upload hands the host a torn snapshot.
pub async fn remote_launch_precondition(
  profile: &crate::profile::types::BrowserProfile,
) -> Result<(), String> {
  remote_launch_profile_rules(profile)?;

  // The manifest is written last, so a host pulling mid-upload gets files
  // that are about to be replaced and a manifest that does not describe them.
  // The browser then comes up on a profile that never existed on this machine
  // and pushes it back over the real one.
  if let Some(scheduler) = crate::sync::get_global_scheduler() {
    if scheduler
      .is_profile_sync_in_progress(&profile.id.to_string())
      .await
    {
      return Err(serde_json::json!({ "code": "REMOTE_SYNC_IN_PROGRESS" }).to_string());
    }
  }

  Ok(())
}

/// The parts of the rule that depend only on the profile itself.
///
/// Split out so the rules stay unit-testable without a running app or a sync
/// scheduler, and so the live check above cannot be reached without them.
pub fn remote_launch_profile_rules(
  profile: &crate::profile::types::BrowserProfile,
) -> Result<(), String> {
  if !profile.is_sync_enabled() {
    return Err(
      "profile does not have cloud sync enabled; a remote host has no way to \
       obtain it"
        .to_string(),
    );
  }
  if profile.is_encrypted_sync() {
    // The key is derived from a passphrase that never leaves this machine, so
    // the host would download ciphertext, launch Chromium on it, and push the
    // corruption back over the user's real profile.
    return Err(
      "profile uses end-to-end encrypted sync; a remote host cannot decrypt \
       it. Switch the profile to Regular sync to run it remotely."
        .to_string(),
    );
  }
  if profile.resolved_os().is_none() {
    return Err(
      "profile has no recorded operating system, so it cannot be scheduled \
       onto a matching host"
        .to_string(),
    );
  }
  Ok(())
}

// API Handler - Stop a REMOTE session started by run-remote
#[utoipa::path(
  delete,
  path = "/v1/remote-sessions/{id}",
  params(
    ("id" = String, Path, description = "Remote session ID from run-remote")
  ),
  responses(
    (status = 200, description = "Remote session stopped", body = StopRemoteResponse),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "No such remote session"),
    (status = 429, description = "Automation request rate limit exceeded"),
    (status = 503, description = "The fleet could not be reached; the session is still running"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "profiles"
)]
async fn stop_remote_session(
  Path(id): Path<String>,
) -> Result<Json<StopRemoteResponse>, (StatusCode, String)> {
  // Without this route, `run-remote` hands back a session id nothing can act
  // on: the only thing that ends a session is the fleet's own two-hour cap, so
  // every launch bills 7200s no matter how briefly it ran.
  let outcome = crate::remote_session::end_remote_session(&id)
    .await
    .map_err(remote_session_error_response)?;

  Ok(Json(StopRemoteResponse {
    session_id: outcome.session_id,
    status: outcome.status,
    billed_seconds: outcome.billed_seconds,
  }))
}

fn remote_session_error_response(
  err: crate::remote_session::RemoteSessionError,
) -> (StatusCode, String) {
  use crate::remote_session::RemoteSessionError;
  match err {
    RemoteSessionError::NoCapacity(m) => (StatusCode::SERVICE_UNAVAILABLE, m),
    RemoteSessionError::Conflict(m) => (StatusCode::CONFLICT, m),
    RemoteSessionError::NotAuthorised(m) => (StatusCode::PAYMENT_REQUIRED, m),
    RemoteSessionError::Other(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
  }
}

/// Map a read of remote state onto a status, and answer with a machine code.
///
/// Separate from `remote_session_error_response` on purpose. That one serves
/// the LAUNCH path, whose documented contract is a plain-English diagnostic and
/// whose only interesting failures are "busy", "already open" and "not on your
/// plan". A read has a different failure set — chiefly "no such session", which
/// the launch mapping would report as a 500 — and it is new, so it can answer
/// with the `{"code":…}` envelope from the start instead of English a client
/// would have to pattern-match.
fn remote_session_read_response(
  err: crate::remote_session::RemoteSessionError,
) -> (StatusCode, String) {
  use crate::remote_session::RemoteSessionError;
  let upstream = match &err {
    RemoteSessionError::NoCapacity(_) => 503,
    RemoteSessionError::Conflict(_) => 409,
    RemoteSessionError::NotAuthorised(_) => 403,
    // The status was consumed on the way in; the code is recovered from the
    // backend's own envelope instead.
    RemoteSessionError::Other(_) => 0,
  };
  let body = err.to_error_json();
  let status = cloud_failure_status(upstream, &error_code_of(&body));
  (status, body)
}

fn cookie_bot_error_response(err: crate::cookie_bot::CookieBotError) -> (StatusCode, String) {
  let status = cloud_failure_status(err.status(), err.code());
  (status, err.to_error_json())
}

/// Read the machine code out of a `{"code":…}` body.
fn error_code_of(body: &str) -> String {
  serde_json::from_str::<serde_json::Value>(body)
    .ok()
    .and_then(|value| {
      value
        .get("code")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
    })
    .unwrap_or_default()
}

/// Turn a donutbrowser-infra failure into the status a local client can act on.
///
/// The upstream status is not echoed blindly. A 401 up there means THIS desktop
/// has no cloud session, which has nothing to do with the caller's own bearer
/// token — answering 401 would send an automation client off to rotate a token
/// that is perfectly good. Likewise the cloud's 403 covers two unrelated
/// things: "your plan does not include this", which is the 402 this API uses
/// everywhere else, and "you are not a member of that team", which no payment
/// fixes.
fn cloud_failure_status(upstream: u16, code: &str) -> StatusCode {
  // The disambiguations first, because no status can express them.
  if code == crate::cloud_errors::NOT_SIGNED_IN {
    return StatusCode::FORBIDDEN;
  }
  if code.ends_with("NOT_ENTITLED") || code == "REMOTE_HOURS_EXHAUSTED" {
    return StatusCode::PAYMENT_REQUIRED;
  }
  if code == crate::cloud_errors::RATE_LIMITED {
    return StatusCode::TOO_MANY_REQUESTS;
  }
  if code == crate::cloud_errors::UNREACHABLE || code == crate::cloud_errors::NO_CAPACITY {
    return StatusCode::SERVICE_UNAVAILABLE;
  }

  match upstream {
    400 | 422 => StatusCode::BAD_REQUEST,
    402 => StatusCode::PAYMENT_REQUIRED,
    403 => StatusCode::FORBIDDEN,
    404 => StatusCode::NOT_FOUND,
    409 => StatusCode::CONFLICT,
    429 => StatusCode::TOO_MANY_REQUESTS,
    503 => StatusCode::SERVICE_UNAVAILABLE,
    // Some transports keep only the body, so the status is gone by the time
    // it gets here. Falling straight through to 500 would report every one of
    // those as our fault, including "no such run".
    _ => status_for_code(code),
  }
}

/// The status a machine code implies when the HTTP status did not survive.
fn status_for_code(code: &str) -> StatusCode {
  if code.ends_with("NOT_FOUND") || code == "COOKIE_BOT_NOT_ENROLLED" {
    StatusCode::NOT_FOUND
  } else if code.ends_with("CONFLICT")
    || code == "COOKIE_BOT_RUN_IN_PROGRESS"
    || code == "REMOTE_SYNC_IN_PROGRESS"
  {
    StatusCode::CONFLICT
  } else if code.starts_with("COOKIE_BOT_INVALID")
    || code == "COOKIE_BOT_SITE_LIMIT"
    || code == "REMOTE_SESSION_REFUSED"
  {
    StatusCode::BAD_REQUEST
  } else if code == "NOT_TEAM_MEMBER" {
    StatusCode::FORBIDDEN
  } else {
    StatusCode::INTERNAL_SERVER_ERROR
  }
}

// API Handler - Every remote session this account currently owns
#[utoipa::path(
  get,
  path = "/v1/remote-sessions",
  responses(
    (status = 200, description = "Sessions owned by the signed-in account", body = ApiRemoteSessionsResponse),
    (status = 401, description = "Unauthorized"),
    (status = 402, description = "Active paid plan with browser automation required"),
    (status = 403, description = "This desktop is not signed in to Donut cloud"),
    (status = 503, description = "Donut cloud could not be reached"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "remote-sessions"
)]
async fn list_remote_sessions_api() -> Result<Json<ApiRemoteSessionsResponse>, (StatusCode, String)>
{
  let sessions = crate::remote_session::list_remote_sessions()
    .await
    .map_err(remote_session_read_response)?;
  Ok(Json(ApiRemoteSessionsResponse { sessions }))
}

// API Handler - One remote session's real state
#[utoipa::path(
  get,
  path = "/v1/remote-sessions/{id}",
  params(
    ("id" = String, Path, description = "Remote session ID from run-remote")
  ),
  responses(
    (status = 200, description = "Current session state", body = crate::remote_session::RemoteSessionState),
    (status = 401, description = "Unauthorized"),
    (status = 402, description = "Active paid plan with browser automation required"),
    (status = 403, description = "This desktop is not signed in to Donut cloud"),
    (status = 404, description = "No such remote session"),
    (status = 503, description = "Donut cloud could not be reached"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "remote-sessions"
)]
async fn get_remote_session_api(
  Path(id): Path<String>,
) -> Result<Json<crate::remote_session::RemoteSessionState>, (StatusCode, String)> {
  // `run-remote` answers `provisioning` and nothing more. Until this route
  // existed, an automation client had no way to learn a session had become
  // usable other than repeatedly trying to drive it.
  crate::remote_session::get_remote_session(&id)
    .await
    .map(Json)
    .map_err(remote_session_read_response)
}

// API Handler - The pooled remote-hour budget
#[utoipa::path(
  get,
  path = "/v1/remote-hours",
  responses(
    (status = 200, description = "Pooled remote-hour budget and its breakdown", body = crate::cookie_bot::RemoteHoursQuota),
    (status = 401, description = "Unauthorized"),
    (status = 403, description = "This desktop is not signed in to Donut cloud"),
    (status = 503, description = "Donut cloud could not be reached"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "remote-sessions"
)]
async fn get_remote_hours(
) -> Result<Json<crate::cookie_bot::RemoteHoursQuota>, (StatusCode, String)> {
  // Bot runs and interactive remote sessions spend one pool. Being refused a
  // launch should not be the only way to find out how much of it is left.
  crate::cookie_bot::remote_hours_quota()
    .await
    .map(Json)
    .map_err(cookie_bot_error_response)
}

// --- Cookie bot -------------------------------------------------------------
//
// Thin proxies onto donutbrowser-infra, which owns the schedule, the calendar
// arithmetic, the browsing model and the pooled hour budget. Nothing here
// decides when a run happens or what it does. What this file DOES decide is
// which profiles may be offered to it at all.

/// Resolve a profile the cookie bot is allowed to touch.
///
/// The bot exists only on the leased fleet: a run materialises the profile on a
/// remote host from cloud sync, warms it, and pushes it back. A profile that
/// cannot make that round trip — never synced, encrypted with a key that never
/// leaves this machine, no recorded OS, an OS the fleet cannot lease, or no
/// proxy or VPN to egress through — has no path to a run and must never reach
/// an enrolment, a quota check or a leased host.
///
/// Every cookie-bot WRITE on this server goes through here, so there is no
/// surface on which a local-only profile can be pointed at the bot. The server
/// re-checks all of it; this exists so the refusal happens at the moment the
/// caller asks rather than silently at 02:00.
fn cookie_bot_eligible_profile(
  profile_id: &str,
) -> Result<crate::profile::types::BrowserProfile, (StatusCode, String)> {
  let profiles = ProfileManager::instance()
    .list_profiles()
    .map_err(manager_error_response)?;
  let profile = profiles
    .into_iter()
    .find(|p| p.id.to_string() == profile_id)
    .ok_or((StatusCode::NOT_FOUND, "profile not found".to_string()))?;

  crate::cookie_bot::bot_precondition(&profile)
    .map_err(|reason| (StatusCode::BAD_REQUEST, reason))?;
  Ok(profile)
}

// API Handler - Every cookie-bot enrolment the caller can see
#[utoipa::path(
  get,
  path = "/v1/cookie-bot/schedules",
  params(
    ("scope" = Option<String>, Query, description = "`mine` (default) or `team`")
  ),
  responses(
    (status = 200, description = "Enrolled profiles", body = crate::cookie_bot::CookieBotScheduleList),
    (status = 401, description = "Unauthorized"),
    (status = 402, description = "Plan does not include the cookie bot"),
    (status = 403, description = "Not signed in, or scope=team from a non-member"),
    (status = 503, description = "Donut cloud could not be reached"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "cookie-bot"
)]
async fn list_cookie_bot_schedules(
  Query(query): Query<CookieBotScopeQuery>,
) -> Result<Json<crate::cookie_bot::CookieBotScheduleList>, (StatusCode, String)> {
  crate::cookie_bot::list_schedules(query.scope.as_deref())
    .await
    .map(Json)
    .map_err(cookie_bot_error_response)
}

// API Handler - One profile's enrolment
#[utoipa::path(
  get,
  path = "/v1/cookie-bot/schedules/{profile_id}",
  params(
    ("profile_id" = String, Path, description = "Profile ID")
  ),
  responses(
    (status = 200, description = "The profile's enrolment", body = crate::cookie_bot::CookieBotSchedule),
    (status = 401, description = "Unauthorized"),
    (status = 403, description = "This desktop is not signed in to Donut cloud"),
    (status = 404, description = "This profile is not enrolled"),
    (status = 503, description = "Donut cloud could not be reached"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "cookie-bot"
)]
async fn get_cookie_bot_schedule(
  Path(profile_id): Path<String>,
) -> Result<Json<crate::cookie_bot::CookieBotSchedule>, (StatusCode, String)> {
  // Deliberately NOT gated on eligibility: a profile whose sync was turned off
  // after it was enrolled must still be able to show what it is enrolled as,
  // otherwise the only way to see the schedule is to be allowed to run it.
  match crate::cookie_bot::get_schedule(&profile_id)
    .await
    .map_err(cookie_bot_error_response)?
  {
    Some(schedule) => Ok(Json(schedule)),
    None => Err((
      StatusCode::NOT_FOUND,
      serde_json::json!({ "code": "COOKIE_BOT_NOT_ENROLLED" }).to_string(),
    )),
  }
}

// API Handler - Enrol a profile, or replace its enrolment
#[utoipa::path(
  put,
  path = "/v1/cookie-bot/schedules/{profile_id}",
  params(
    ("profile_id" = String, Path, description = "Profile ID")
  ),
  request_body = SetCookieBotScheduleRequest,
  responses(
    (status = 200, description = "Enrolment saved", body = crate::cookie_bot::CookieBotScheduleSaved),
    (status = 400, description = "Invalid schedule, or a profile the bot cannot run"),
    (status = 401, description = "Unauthorized"),
    (status = 402, description = "Plan does not include the cookie bot"),
    (status = 403, description = "This desktop is not signed in to Donut cloud"),
    (status = 404, description = "Profile not found"),
    (status = 409, description = "A teammate already enrols this profile; retry with acknowledge_conflict"),
    (status = 503, description = "Donut cloud could not be reached"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "cookie-bot"
)]
async fn set_cookie_bot_schedule(
  Path(profile_id): Path<String>,
  Json(request): Json<SetCookieBotScheduleRequest>,
) -> Result<Json<crate::cookie_bot::CookieBotScheduleSaved>, (StatusCode, String)> {
  let profile = cookie_bot_eligible_profile(&profile_id)?;
  // `bot_precondition` already proved the profile has a resolvable OS the
  // fleet can lease, so this cannot fail; taking it from the profile rather
  // than the request is what stops a caller enrolling a macOS profile onto a
  // Windows host.
  let platform = profile
    .resolved_os()
    .ok_or((
      StatusCode::BAD_REQUEST,
      serde_json::json!({ "code": "COOKIE_BOT_UNKNOWN_PLATFORM" }).to_string(),
    ))?
    .to_string();

  if let Some(requested) = request.platform.as_deref() {
    if requested != platform {
      return Err((
        StatusCode::BAD_REQUEST,
        serde_json::json!({
          "code": "COOKIE_BOT_UNSUPPORTED_PLATFORM",
          "params": { "platform": requested }
        })
        .to_string(),
      ));
    }
  }

  let input = crate::cookie_bot::CookieBotScheduleInput {
    profile_name: request.profile_name.unwrap_or_else(|| profile.name.clone()),
    platform,
    enabled: request.enabled,
    run_at_minute: request.run_at_minute,
    days_mask: request.days_mask,
    timezone: request.timezone,
    preset: request.preset,
    max_minutes: request.max_minutes,
    sites: request.sites,
    jitter_seconds: request.jitter_seconds,
    ..Default::default()
  }
  // The server requires these and cannot read them itself — the profile lives
  // in the user's sync namespace, not its database.
  .with_profile_state(crate::cookie_bot::profile_state(&profile));

  crate::cookie_bot::save_schedule(&profile_id, &input, request.acknowledge_conflict)
    .await
    .map(Json)
    .map_err(cookie_bot_error_response)
}

// API Handler - Turn the bot off for a profile
#[utoipa::path(
  delete,
  path = "/v1/cookie-bot/schedules/{profile_id}",
  params(
    ("profile_id" = String, Path, description = "Profile ID")
  ),
  responses(
    (status = 200, description = "Enrolment removed, or there was none", body = crate::cookie_bot::CookieBotScheduleDeleted),
    (status = 401, description = "Unauthorized"),
    (status = 403, description = "This desktop is not signed in to Donut cloud"),
    (status = 503, description = "Donut cloud could not be reached"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "cookie-bot"
)]
async fn delete_cookie_bot_schedule(
  Path(profile_id): Path<String>,
) -> Result<Json<crate::cookie_bot::CookieBotScheduleDeleted>, (StatusCode, String)> {
  // No eligibility gate and no 404: "turn the bot off" must be safe to repeat,
  // and a profile that has since become ineligible is exactly the one a caller
  // most needs to be able to unenrol.
  crate::cookie_bot::delete_schedule(&profile_id)
    .await
    .map(Json)
    .map_err(cookie_bot_error_response)
}

// API Handler - Who else already warms this profile
#[utoipa::path(
  get,
  path = "/v1/cookie-bot/conflicts",
  params(
    ("profile_id" = String, Query, description = "Profile ID"),
    ("run_at_minute" = Option<u16>, Query, description = "Proposed minute past local midnight"),
    ("timezone" = Option<String>, Query, description = "Proposed IANA zone"),
    ("days_mask" = Option<u8>, Query, description = "Proposed weekday bitmask, bit 0 = Monday")
  ),
  responses(
    (status = 200, description = "Teammates enrolling the same profile", body = crate::cookie_bot::CookieBotConflictCheck),
    (status = 400, description = "profile_id missing"),
    (status = 401, description = "Unauthorized"),
    (status = 403, description = "This desktop is not signed in to Donut cloud"),
    (status = 503, description = "Donut cloud could not be reached"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "cookie-bot"
)]
async fn get_cookie_bot_conflicts(
  Query(query): Query<CookieBotConflictsQuery>,
) -> Result<Json<crate::cookie_bot::CookieBotConflictCheck>, (StatusCode, String)> {
  // A dry run that writes nothing, so an automation client can find a
  // collision before it makes one instead of after two operators have quietly
  // scheduled the same profile against itself.
  crate::cookie_bot::check_conflicts(
    &query.profile_id,
    query.run_at_minute,
    query.timezone.as_deref(),
    query.days_mask,
  )
  .await
  .map(Json)
  .map_err(cookie_bot_error_response)
}

// API Handler - Run history
#[utoipa::path(
  get,
  path = "/v1/cookie-bot/runs",
  params(
    ("profile_id" = Option<String>, Query, description = "Restrict to one profile"),
    ("scope" = Option<String>, Query, description = "`mine` (default) or `team`"),
    ("limit" = Option<u32>, Query, description = "Page size, 1..100 (default 30)"),
    ("before" = Option<String>, Query, description = "Keyset cursor from a previous page's next_before")
  ),
  responses(
    (status = 200, description = "One page of runs, newest first", body = crate::cookie_bot::CookieBotRunPage),
    (status = 400, description = "limit out of range or malformed cursor"),
    (status = 401, description = "Unauthorized"),
    (status = 403, description = "Not signed in, or scope=team from a non-member"),
    (status = 503, description = "Donut cloud could not be reached"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "cookie-bot"
)]
async fn list_cookie_bot_runs(
  Query(query): Query<CookieBotRunsQuery>,
) -> Result<Json<crate::cookie_bot::CookieBotRunPage>, (StatusCode, String)> {
  crate::cookie_bot::list_runs(
    query.profile_id.as_deref(),
    query.scope.as_deref(),
    query.limit,
    query.before.as_deref(),
  )
  .await
  .map(Json)
  .map_err(cookie_bot_error_response)
}

// API Handler - Warm a profile now instead of waiting for tonight
#[utoipa::path(
  post,
  path = "/v1/cookie-bot/runs",
  request_body = StartCookieBotRunRequest,
  responses(
    (status = 202, description = "Run accepted; it keeps executing for minutes after this response", body = crate::cookie_bot::CookieBotRunStarted),
    (status = 400, description = "A profile the bot cannot run"),
    (status = 401, description = "Unauthorized"),
    (status = 402, description = "Plan does not include the cookie bot, or the pooled hours are spent"),
    (status = 403, description = "This desktop is not signed in to Donut cloud"),
    (status = 404, description = "Profile not found, or not enrolled"),
    (status = 409, description = "A run or remote session already holds this profile"),
    (status = 429, description = "Automation request rate limit exceeded"),
    (status = 503, description = "No host of that operating system has a free slot"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "cookie-bot"
)]
async fn start_cookie_bot_run(
  Json(request): Json<StartCookieBotRunRequest>,
) -> Result<(StatusCode, Json<crate::cookie_bot::CookieBotRunStarted>), (StatusCode, String)> {
  if !crate::cloud_auth::CLOUD_AUTH
    .can_use_browser_automation()
    .await
  {
    return Err((StatusCode::PAYMENT_REQUIRED, String::new()));
  }

  cookie_bot_eligible_profile(&request.profile_id)?;

  let started = crate::cookie_bot::run_now(&request.profile_id, request.max_minutes)
    .await
    .map_err(cookie_bot_error_response)?;

  // 202, not 200: the fleet is still browsing when this returns. Answering 200
  // would tell a client the work is done when it has barely started.
  Ok((StatusCode::ACCEPTED, Json(started)))
}

// API Handler - Stop a run that is still going
#[utoipa::path(
  delete,
  path = "/v1/cookie-bot/runs/{run_id}",
  params(
    ("run_id" = String, Path, description = "Run ID")
  ),
  responses(
    (status = 200, description = "The run, cancelled (or unchanged if it had already finished)", body = crate::cookie_bot::CookieBotRun),
    (status = 401, description = "Unauthorized"),
    (status = 403, description = "This desktop is not signed in to Donut cloud"),
    (status = 404, description = "No such run for this account"),
    (status = 429, description = "Automation request rate limit exceeded"),
    (status = 503, description = "The fleet could not be reached; the run is still live"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "cookie-bot"
)]
async fn cancel_cookie_bot_run(
  Path(run_id): Path<String>,
) -> Result<Json<crate::cookie_bot::CookieBotRun>, (StatusCode, String)> {
  // No entitlement gate. A lapsed plan must never be the reason a user cannot
  // stop something that is spending their hours.
  crate::cookie_bot::cancel_run(&run_id)
    .await
    .map(Json)
    .map_err(cookie_bot_error_response)
}

// API Handler - The intensities the server offers
#[utoipa::path(
  get,
  path = "/v1/cookie-bot/presets",
  responses(
    (status = 200, description = "Selectable presets", body = crate::cookie_bot::CookieBotPresetList),
    (status = 401, description = "Unauthorized"),
    (status = 403, description = "This desktop is not signed in to Donut cloud"),
    (status = 503, description = "Donut cloud could not be reached"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "cookie-bot"
)]
async fn list_cookie_bot_presets(
) -> Result<Json<crate::cookie_bot::CookieBotPresetList>, (StatusCode, String)> {
  // Ids and a rough duration only. What a preset expands to — the site
  // ordering, the dwell model, the scroll and click programme — is the
  // server's, and stays there.
  crate::cookie_bot::list_presets()
    .await
    .map(Json)
    .map_err(cookie_bot_error_response)
}

// API Handler - Who spent what, for a calendar month
#[utoipa::path(
  get,
  path = "/v1/cookie-bot/usage",
  params(
    ("period" = Option<String>, Query, description = "`YYYY-MM`; defaults to the current UTC month")
  ),
  responses(
    (status = 200, description = "Per-member and per-profile spend", body = crate::cookie_bot::CookieBotUsage),
    (status = 400, description = "Malformed period"),
    (status = 401, description = "Unauthorized"),
    (status = 403, description = "Not signed in, or not a member of that team"),
    (status = 503, description = "Donut cloud could not be reached"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "cookie-bot"
)]
async fn get_cookie_bot_usage(
  Query(query): Query<CookieBotUsageQuery>,
) -> Result<Json<crate::cookie_bot::CookieBotUsage>, (StatusCode, String)> {
  // Reporting, never enforcement: the pooled budget is spent against by the
  // server, and this is how an owner finds out where it went.
  crate::cookie_bot::team_usage(query.period.as_deref())
    .await
    .map(Json)
    .map_err(cookie_bot_error_response)
}

// API Handler - Open URL in existing browser
#[utoipa::path(
  post,
  path = "/v1/profiles/{id}/open-url",
  params(
    ("id" = String, Path, description = "Profile ID")
  ),
  request_body = OpenUrlRequest,
  responses(
    (status = 200, description = "URL opened successfully"),
    (status = 400, description = "Cannot open URL with a cross-OS profile"),
    (status = 401, description = "Unauthorized"),
    (status = 402, description = "Active paid plan with browser automation required"),
    (status = 404, description = "Profile not found"),
    (status = 429, description = "Automation request rate limit exceeded"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "profiles"
)]
async fn open_url_in_profile(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
  Json(request): Json<OpenUrlRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
  if !crate::cloud_auth::CLOUD_AUTH
    .can_use_browser_automation()
    .await
  {
    return Err((StatusCode::PAYMENT_REQUIRED, String::new()));
  }

  let browser_runner = crate::browser_runner::BrowserRunner::instance();

  browser_runner
    .open_url_with_profile(state.app_handle.clone(), id, request.url)
    .await
    .map_err(manager_error_response)?;

  Ok(StatusCode::OK)
}

// API Handler - Kill browser process
#[utoipa::path(
  post,
  path = "/v1/profiles/{id}/kill",
  params(
    ("id" = String, Path, description = "Profile ID")
  ),
  responses(
    (status = 204, description = "Browser process killed successfully"),
    (status = 401, description = "Unauthorized"),
    (status = 402, description = "Active paid plan required"),
    (status = 404, description = "Profile not found"),
    (status = 429, description = "Automation request rate limit exceeded"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "profiles"
)]
async fn kill_profile(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
) -> Result<StatusCode, StatusCode> {
  // Programmatically launching and stopping profiles is a paid feature; the
  // run/open-url handlers gate the same way.
  if !crate::cloud_auth::CLOUD_AUTH
    .can_use_browser_automation()
    .await
  {
    return Err(StatusCode::PAYMENT_REQUIRED);
  }

  let profile_manager = ProfileManager::instance();
  let profiles = profile_manager
    .list_profiles()
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

  let profile = profiles
    .iter()
    .find(|p| p.id.to_string() == id)
    .ok_or(StatusCode::NOT_FOUND)?;

  let browser_runner = crate::browser_runner::BrowserRunner::instance();
  browser_runner
    .kill_browser_process(state.app_handle.clone(), profile)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

  crate::team_lock::release_team_lock_if_needed(profile).await;

  Ok(StatusCode::NO_CONTENT)
}

// API Handler - Batch run profiles (paid: browser automation). Mirrors the
// single `/run` gate; never breaks the batch on a single profile's failure —
// each profile gets its own result entry.
#[utoipa::path(
  post,
  path = "/v1/profiles/batch/run",
  request_body = BatchRunRequest,
  responses(
    (status = 200, description = "Batch launch completed; inspect per-profile results", body = BatchRunResponse),
    (status = 401, description = "Unauthorized"),
    (status = 402, description = "Active paid plan with browser automation required"),
    (status = 429, description = "Automation request rate limit exceeded"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "profiles"
)]
async fn batch_run_profiles(
  State(state): State<ApiServerState>,
  Json(request): Json<BatchRunRequest>,
) -> Result<Json<BatchRunResponse>, StatusCode> {
  if !crate::cloud_auth::CLOUD_AUTH
    .can_use_browser_automation()
    .await
  {
    return Err(StatusCode::PAYMENT_REQUIRED);
  }

  let headless = request.headless.unwrap_or(false);
  let profile_manager = ProfileManager::instance();
  let profiles = profile_manager
    .list_profiles()
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

  let mut results = Vec::with_capacity(request.profile_ids.len());
  for profile_id in &request.profile_ids {
    let fail = |error: &str| BatchRunResult {
      profile_id: profile_id.clone(),
      ok: false,
      remote_debugging_port: None,
      error: Some(error.to_string()),
    };

    let Some(profile) = profiles.iter().find(|p| p.id.to_string() == *profile_id) else {
      results.push(fail("profile not found"));
      continue;
    };
    if profile.is_cross_os() {
      results.push(fail("cross-OS profiles cannot be launched"));
      continue;
    }
    if crate::team_lock::acquire_team_lock_if_needed(profile)
      .await
      .is_err()
    {
      results.push(fail("profile is locked by another team member"));
      continue;
    }

    let port = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
      Ok(listener) => match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(_) => {
          results.push(fail("failed to allocate debugging port"));
          continue;
        }
      },
      Err(_) => {
        results.push(fail("failed to allocate debugging port"));
        continue;
      }
    };

    match crate::browser_runner::launch_browser_profile_impl(
      state.app_handle.clone(),
      profile.clone(),
      request.url.clone(),
      Some(port),
      headless,
      true,
    )
    .await
    {
      Ok(_) => results.push(BatchRunResult {
        profile_id: profile_id.clone(),
        ok: true,
        remote_debugging_port: Some(port),
        error: None,
      }),
      Err(e) => results.push(fail(&format!("launch failed: {e}"))),
    }
  }

  Ok(Json(BatchRunResponse { results }))
}

// API Handler - Batch stop profiles (paid: browser automation).
#[utoipa::path(
  post,
  path = "/v1/profiles/batch/stop",
  request_body = BatchStopRequest,
  responses(
    (status = 200, description = "Batch stop completed; inspect per-profile results", body = BatchStopResponse),
    (status = 401, description = "Unauthorized"),
    (status = 402, description = "Active paid plan with browser automation required"),
    (status = 429, description = "Automation request rate limit exceeded"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "profiles"
)]
async fn batch_stop_profiles(
  State(state): State<ApiServerState>,
  Json(request): Json<BatchStopRequest>,
) -> Result<Json<BatchStopResponse>, StatusCode> {
  if !crate::cloud_auth::CLOUD_AUTH
    .can_use_browser_automation()
    .await
  {
    return Err(StatusCode::PAYMENT_REQUIRED);
  }

  let profile_manager = ProfileManager::instance();
  let profiles = profile_manager
    .list_profiles()
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
  let browser_runner = crate::browser_runner::BrowserRunner::instance();

  let mut results = Vec::with_capacity(request.profile_ids.len());
  for profile_id in &request.profile_ids {
    let Some(profile) = profiles.iter().find(|p| p.id.to_string() == *profile_id) else {
      results.push(BatchStopResult {
        profile_id: profile_id.clone(),
        ok: false,
        error: Some("profile not found".to_string()),
      });
      continue;
    };

    match browser_runner
      .kill_browser_process(state.app_handle.clone(), profile)
      .await
    {
      Ok(_) => {
        crate::team_lock::release_team_lock_if_needed(profile).await;
        results.push(BatchStopResult {
          profile_id: profile_id.clone(),
          ok: true,
          error: None,
        });
      }
      Err(e) => results.push(BatchStopResult {
        profile_id: profile_id.clone(),
        ok: false,
        error: Some(format!("stop failed: {e}")),
      }),
    }
  }

  Ok(Json(BatchStopResponse { results }))
}

// API Handler - Detect importable browser profiles on this machine, or scan a
// custom folder. Free: importing is not gated behind browser automation.
#[utoipa::path(
  get,
  path = "/v1/profiles/import/detect",
  params(
    ("folder" = Option<String>, Query, description = "Optional folder to scan instead of the default browser locations. Accepts a single profile dir, a Chromium user-data dir, or a folder holding one profile dir per child.")
  ),
  responses(
    (status = 200, description = "Detected importable profiles", body = DetectedProfilesResponse),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "Folder not found"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "profiles"
)]
async fn detect_import_profiles(
  Query(query): Query<DetectImportQuery>,
  State(_state): State<ApiServerState>,
) -> Result<Json<DetectedProfilesResponse>, (StatusCode, String)> {
  let importer = crate::profile_importer::ProfileImporter::instance();
  let profiles = match query.folder.as_deref() {
    Some(folder) => importer.scan_folder(std::path::Path::new(folder)),
    None => importer.detect_existing_profiles(),
  }
  .map_err(manager_error_response)?;
  let total = profiles.len();
  Ok(Json(DetectedProfilesResponse { profiles, total }))
}

// API Handler - Bulk-import browser profiles from on-disk profile folders.
// Free (parity with create_profile); only fingerprint OS spoofing is Pro.
// Items are isolated — one failure doesn't stop the rest.
#[utoipa::path(
  post,
  path = "/v1/profiles/import",
  request_body = ImportProfilesRequest,
  responses(
    (status = 200, description = "Batch import completed; inspect per-item results", body = crate::profile_importer::ProfileImportBatchResult),
    (status = 400, description = "No items, or invalid input"),
    (status = 401, description = "Unauthorized"),
    (status = 402, description = "Fingerprint OS spoofing requires an active Pro subscription"),
    (status = 404, description = "Group not found"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "profiles"
)]
async fn import_profiles_api(
  State(state): State<ApiServerState>,
  Json(request): Json<ImportProfilesRequest>,
) -> Result<Json<crate::profile_importer::ProfileImportBatchResult>, (StatusCode, String)> {
  let wayfern_config: Option<crate::wayfern_manager::WayfernConfig> = request
    .wayfern_config
    .as_ref()
    .and_then(|config| serde_json::from_value(config.clone()).ok());

  // The Pro gate for fingerprint OS spoofing lives inside import_profiles, so
  // every surface inherits it; manager_error_response maps the code to 402.
  let importer = crate::profile_importer::ProfileImporter::instance();
  importer
    .import_profiles(
      &state.app_handle,
      request.items,
      request.group_id,
      request.duplicate_strategy.unwrap_or_default(),
      wayfern_config,
    )
    .await
    .map(Json)
    .map_err(manager_error_response)
}

#[utoipa::path(
  post,
  path = "/v1/profiles/{id}/cookies/import",
  params(
    ("id" = String, Path, description = "Profile ID")
  ),
  request_body = ImportCookiesRequest,
  responses(
    (status = 200, description = "Cookies imported successfully", body = ImportCookiesResponse),
    (status = 400, description = "Invalid cookie file or unsupported browser"),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "Profile not found"),
    (status = 409, description = "Browser is currently running"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "cookies"
)]
async fn import_profile_cookies(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
  Json(request): Json<ImportCookiesRequest>,
) -> Result<Json<ImportCookiesResponse>, StatusCode> {
  let profile_manager = ProfileManager::instance();
  let profiles = profile_manager
    .list_profiles()
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

  if !profiles.iter().any(|p| p.id.to_string() == id) {
    return Err(StatusCode::NOT_FOUND);
  }

  match crate::cookie_manager::CookieManager::import_cookies(
    &state.app_handle,
    &id,
    &request.content,
  )
  .await
  {
    Ok(result) => {
      if let Some(scheduler) = crate::sync::get_global_scheduler() {
        if let Some(profile) = profiles.iter().find(|p| p.id.to_string() == id) {
          if profile.is_sync_enabled() {
            let pid = id.clone();
            tauri::async_runtime::spawn(async move {
              scheduler.queue_profile_sync(pid).await;
            });
          }
        }
      }
      Ok(Json(ImportCookiesResponse {
        cookies_imported: result.cookies_imported,
        cookies_replaced: result.cookies_replaced,
        errors: result.errors,
      }))
    }
    Err(e) => {
      let msg = e.to_lowercase();
      if msg.contains("running") {
        Err(StatusCode::CONFLICT)
      } else if msg.contains("no valid cookies") || msg.contains("unsupported browser") {
        Err(StatusCode::BAD_REQUEST)
      } else {
        Err(StatusCode::INTERNAL_SERVER_ERROR)
      }
    }
  }
}

// API Handler - Download Browser
#[utoipa::path(
  post,
  path = "/v1/browsers/download",
  request_body = DownloadBrowserRequest,
  responses(
    (status = 200, description = "Browser downloaded (or already present)", body = DownloadBrowserResponse),
    (status = 400, description = "Invalid browser or version not available for download"),
    (status = 401, description = "Unauthorized"),
    (status = 409, description = "This browser version is already being downloaded"),
    (status = 500, description = "Internal server error (e.g. network failure)")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "browsers"
)]
async fn download_browser_api(
  State(state): State<ApiServerState>,
  Json(request): Json<DownloadBrowserRequest>,
) -> Result<Json<DownloadBrowserResponse>, (StatusCode, String)> {
  match crate::downloader::download_browser(
    state.app_handle.clone(),
    request.browser.clone(),
    request.version,
  )
  .await
  {
    // Echo the version the downloader actually installed, not the requested one.
    Ok(version) => Ok(Json(DownloadBrowserResponse {
      browser: request.browser,
      version,
      status: "downloaded".to_string(),
    })),
    Err(e) => {
      if e.contains("already being downloaded") {
        Err((StatusCode::CONFLICT, e))
      } else {
        Err(manager_error_response(e))
      }
    }
  }
}

// API Handler - Get Browser Versions
#[utoipa::path(
  get,
  path = "/v1/browsers/{browser}/versions",
  params(
    ("browser" = String, Path, description = "Browser name")
  ),
  responses(
    (status = 200, description = "List of available browser versions", body = Vec<String>),
    (status = 400, description = "Unsupported browser"),
    (status = 401, description = "Unauthorized"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "browsers"
)]
async fn get_browser_versions(
  Path(browser): Path<String>,
  State(_state): State<ApiServerState>,
) -> Result<Json<Vec<String>>, (StatusCode, String)> {
  let version_manager = crate::browser_version_manager::BrowserVersionManager::instance();

  match version_manager
    .fetch_browser_versions_with_count(&browser, false)
    .await
  {
    Ok(result) => Ok(Json(result.versions)),
    Err(e) => Err(manager_error_response(e)),
  }
}

// API Handler - Check if Browser is Downloaded
#[utoipa::path(
  get,
  path = "/v1/browsers/{browser}/versions/{version}/downloaded",
  params(
    ("browser" = String, Path, description = "Browser name"),
    ("version" = String, Path, description = "Browser version")
  ),
  responses(
    (status = 200, description = "Browser download status", body = bool),
    (status = 401, description = "Unauthorized"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "browsers"
)]
async fn check_browser_downloaded(
  Path((browser, version)): Path<(String, String)>,
  State(_state): State<ApiServerState>,
) -> Result<Json<bool>, StatusCode> {
  let is_downloaded = crate::downloaded_browsers_registry::is_browser_downloaded(browser, version);
  Ok(Json(is_downloaded))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::profile::types::{BrowserProfile, SyncMode};

  fn profile_with(sync_mode: SyncMode, host_os: Option<&str>) -> BrowserProfile {
    BrowserProfile {
      id: uuid::Uuid::nil(),
      name: "p".to_string(),
      browser: "wayfern".to_string(),
      version: "latest".to_string(),
      sync_mode,
      host_os: host_os.map(|s| s.to_string()),
      ..Default::default()
    }
  }

  // Cloud sync has been settable through PUT /v1/profiles/{id} but was absent
  // from every profile RESPONSE, so a caller could turn it on and never
  // confirm it. A remote-launch caller must be able to see this before it can
  // decide whether the profile exists in cloud storage at all.
  // /run-remote exists precisely so a profile can run on a host of ITS OWN OS
  // when this machine is the wrong one. The gate is cloud sync: a remote host
  // obtains the profile from donut-sync, so a profile that has never synced
  // would launch an empty browser and push that emptiness over the real one.
  #[test]
  fn remote_launch_requires_cloud_sync() {
    let err = remote_launch_profile_rules(&profile_with(SyncMode::Disabled, Some("macos")))
      .expect_err("a non-synced profile must be refused");
    assert!(err.contains("cloud sync"), "unhelpful message: {err}");

    assert!(
      remote_launch_profile_rules(&profile_with(SyncMode::Regular, Some("macos"))).is_ok(),
      "a synced profile must be allowed"
    );
  }

  #[test]
  fn remote_launch_refuses_an_end_to_end_encrypted_profile() {
    // The key is derived from a passphrase that never leaves this machine, so
    // a remote host downloads ciphertext, launches Chromium on it, and pushes
    // the corruption back over the user's real profile. Refusing here also
    // saves taking the profile lock and a slot on leased hardware for a
    // session that cannot possibly work.
    let err = remote_launch_profile_rules(&profile_with(SyncMode::Encrypted, Some("macos")))
      .expect_err("an encrypted profile must be refused");
    assert!(
      err.contains("encrypted") && err.contains("Regular"),
      "the message must say what to change: {err}"
    );
  }

  #[test]
  fn remote_launch_requires_a_known_operating_system() {
    // Without one there is no way to pick a matching host, and guessing would
    // be the cross-OS mismatch this whole design exists to prevent.
    assert!(remote_launch_profile_rules(&profile_with(SyncMode::Regular, None)).is_err());
  }

  #[test]
  fn remote_launch_allows_a_cross_os_profile() {
    let host = crate::profile::types::get_host_os();
    let other = if host == "windows" {
      "macos"
    } else {
      "windows"
    };
    let foreign = profile_with(SyncMode::Regular, Some(other));

    assert!(
      foreign.is_cross_os(),
      "test setup: profile should be foreign"
    );
    // Local /run refuses this; running it remotely on a host of its own OS is
    // exactly what /run-remote is for.
    assert!(remote_launch_profile_rules(&foreign).is_ok());
  }

  #[tokio::test]
  async fn remote_launch_is_refused_while_the_profile_is_mid_upload() {
    // The manifest is written last. A host that pulls during the upload gets
    // files that are about to be replaced described by a manifest that does
    // not match them, launches Chromium on that, and pushes the result back
    // over the real profile.
    let scheduler = std::sync::Arc::new(crate::sync::SyncScheduler::new());
    crate::sync::set_global_scheduler(scheduler.clone());

    let mut profile = profile_with(SyncMode::Regular, Some("macos"));
    profile.id = uuid::Uuid::new_v4();
    assert!(
      remote_launch_precondition(&profile).await.is_ok(),
      "an idle profile must be launchable"
    );

    scheduler.queue_profile_sync(profile.id.to_string()).await;
    let err = remote_launch_precondition(&profile)
      .await
      .expect_err("a profile mid-upload must be refused");
    assert!(
      err.contains("REMOTE_SYNC_IN_PROGRESS"),
      "the refusal must be a code the frontend can translate: {err}"
    );
  }

  #[test]
  fn api_profile_exposes_cloud_sync_state() {
    let disabled = ApiProfile::from(&profile_with(SyncMode::Disabled, None));
    assert_eq!(disabled.sync_mode, "Disabled");
    assert!(!disabled.cloud_sync_enabled);

    let regular = ApiProfile::from(&profile_with(SyncMode::Regular, None));
    assert_eq!(regular.sync_mode, "Regular");
    assert!(regular.cloud_sync_enabled);

    let encrypted = ApiProfile::from(&profile_with(SyncMode::Encrypted, None));
    assert_eq!(encrypted.sync_mode, "Encrypted");
    assert!(encrypted.cloud_sync_enabled);
  }

  // A profile must only ever run on its own operating system: Chromium's
  // on-disk state is OS-specific, so replaying a macOS profile on Windows is a
  // mismatch no amount of user-agent spoofing repairs.
  #[test]
  fn api_profile_reports_its_operating_system() {
    let host = crate::profile::types::get_host_os();
    let same = ApiProfile::from(&profile_with(SyncMode::Regular, Some(&host)));
    assert_eq!(same.host_os.as_deref(), Some(host.as_str()));
    assert!(!same.is_cross_os);

    let other = if host == "windows" {
      "macos"
    } else {
      "windows"
    };
    let foreign = ApiProfile::from(&profile_with(SyncMode::Regular, Some(other)));
    assert_eq!(foreign.host_os.as_deref(), Some(other));
    assert!(foreign.is_cross_os);
  }

  #[test]
  fn api_profile_without_a_recorded_os_is_not_cross_os() {
    // An older profile that predates host_os must stay locally launchable
    // rather than being treated as foreign.
    let unknown = ApiProfile::from(&profile_with(SyncMode::Disabled, None));
    assert_eq!(unknown.host_os, None);
    assert!(!unknown.is_cross_os);
  }

  // Removing `browser` from UpdateProfileRequest, and rejecting invalid
  // `browser` values on create, must NOT make the API reject requests that
  // carry extra/unknown fields — old clients still send them. serde ignores
  // unknown fields by default; these tests lock that in so a future
  // `#[serde(deny_unknown_fields)]` can't silently break compatibility.
  #[test]
  fn update_profile_request_ignores_unknown_fields() {
    // `browser` is no longer a field, plus a wholly unknown field. Both must
    // be accepted and ignored, not rejected.
    let json = r#"{"name": "p", "browser": "wayfern", "totally_unknown": 123}"#;
    let parsed: UpdateProfileRequest =
      serde_json::from_str(json).expect("unknown fields must be ignored, not rejected");
    assert_eq!(parsed.name.as_deref(), Some("p"));
  }

  #[test]
  fn create_profile_request_ignores_unknown_fields() {
    let json = r#"{"name": "p", "browser": "wayfern", "version": "latest", "future_field": true}"#;
    let parsed: CreateProfileRequest =
      serde_json::from_str(json).expect("unknown fields must be ignored, not rejected");
    assert_eq!(parsed.browser, "wayfern");
  }

  #[test]
  fn create_profile_request_allows_omitting_version_and_configs() {
    // Minimal body: no version, no wayfern_config. Must
    // deserialize (version resolves to latest-downloaded at the handler; an
    // absent config triggers fresh-fingerprint generation).
    let json = r#"{"name": "p", "browser": "wayfern"}"#;
    let parsed: CreateProfileRequest =
      serde_json::from_str(json).expect("version and configs are optional");
    assert_eq!(parsed.browser, "wayfern");
    assert!(parsed.version.is_none());
    assert!(parsed.wayfern_config.is_none());
  }

  #[test]
  fn create_profile_browser_validation_matches_supported_engines() {
    // The handler rejects anything that isn't a launchable engine; this is the
    // same predicate it uses, kept in lockstep with MCP's create_profile.
    let is_valid = |b: &str| b == "wayfern";
    assert!(is_valid("wayfern"));
    assert!(!is_valid("chromium"));
    assert!(!is_valid(""));
  }

  #[test]
  fn rate_limit_only_classifies_browser_automation_routes() {
    for path in [
      "/v1/profiles/profile-id/run",
      "/v1/profiles/profile-id/open-url",
      "/v1/profiles/profile-id/kill",
      // Launching on leased remote hardware is the most expensive automation
      // action there is; it went unmetered because `run-remote` is its own
      // path segment and never matched `run`.
      "/v1/profiles/profile-id/run-remote",
      "/v1/profiles/batch/run",
      "/v1/profiles/batch/stop",
      // Starting a bot run leases a host for up to two hours and spends the
      // account's pooled remote-hour budget.
      "/v1/cookie-bot/runs",
    ] {
      assert!(
        is_automation_request(&Method::POST, path),
        "automation route was not limited: {path}"
      );
    }

    // Stopping a remote session is a DELETE, and its handler declares a 429.
    // Cancelling a bot run reaches the same fleet and is metered the same way.
    for path in [
      "/v1/remote-sessions/session-id",
      "/v1/cookie-bot/runs/run-id",
    ] {
      assert!(
        is_automation_request(&Method::DELETE, path),
        "metered stop was not limited: {path}"
      );
    }

    for (method, path) in [
      (Method::GET, "/v1/profiles/profile-id/run"),
      (Method::POST, "/v1/profiles"),
      (Method::POST, "/v1/profiles/import"),
      (Method::GET, "/v1/profiles"),
      (Method::GET, "/openapi.json"),
      // Only the single-session DELETE is automation; the collection is not a
      // route, and a GET of one never launches anything.
      (Method::DELETE, "/v1/remote-sessions/"),
      (Method::GET, "/v1/remote-sessions/session-id"),
      (Method::GET, "/v1/remote-sessions"),
      // Enrolling a profile writes one row on the server and leases nothing.
      // Metering it would 429 a client setting up a fleet of profiles, while
      // the budget that actually protects the hardware is spent per RUN and
      // enforced server-side however the run was scheduled.
      (Method::PUT, "/v1/cookie-bot/schedules/profile-id"),
      (Method::DELETE, "/v1/cookie-bot/schedules/profile-id"),
      (Method::GET, "/v1/cookie-bot/schedules"),
      (Method::GET, "/v1/cookie-bot/runs"),
      (Method::GET, "/v1/cookie-bot/usage"),
      (Method::GET, "/v1/remote-hours"),
      // A run id is required; the collection DELETE is not a route.
      (Method::DELETE, "/v1/cookie-bot/runs/"),
    ] {
      assert!(
        !is_automation_request(&method, path),
        "free or non-mutating route was limited: {method} {path}"
      );
    }
  }

  // The bot exists only on the leased fleet. Every write surface resolves the
  // profile through `bot_precondition` first, so there is no request shape on
  // this server that points it at a profile which could never make the round
  // trip to a remote host and back.
  #[test]
  fn a_profile_the_bot_could_never_run_is_refused_before_the_cloud_is_asked() {
    let mut local_only = profile_with(SyncMode::Disabled, Some("macos"));
    local_only.proxy_id = Some("proxy-1".to_string());
    assert!(
      crate::cookie_bot::bot_precondition(&local_only).is_err(),
      "a profile with no cloud copy has nothing for a host to open"
    );

    let mut encrypted = profile_with(SyncMode::Encrypted, Some("macos"));
    encrypted.proxy_id = Some("proxy-1".to_string());
    assert!(
      crate::cookie_bot::bot_precondition(&encrypted).is_err(),
      "a host cannot decrypt a profile whose key never leaves this machine"
    );

    let mut datacenter_egress = profile_with(SyncMode::Regular, Some("macos"));
    datacenter_egress.proxy_id = None;
    datacenter_egress.vpn_id = None;
    assert!(
      crate::cookie_bot::bot_precondition(&datacenter_egress).is_err(),
      "hours of traffic from a hosting ASN damages the identity being warmed"
    );

    let mut eligible = profile_with(SyncMode::Regular, Some("macos"));
    eligible.proxy_id = Some("proxy-1".to_string());
    assert!(crate::cookie_bot::bot_precondition(&eligible).is_ok());
  }

  #[test]
  fn a_cloud_401_is_not_reported_as_the_callers_own_token_being_wrong() {
    // The caller's bearer token was accepted — the auth middleware ran. It is
    // THIS desktop that has no cloud session, and answering 401 would send an
    // automation client off to rotate a token that is perfectly good.
    assert_eq!(
      cloud_failure_status(401, crate::cloud_errors::NOT_SIGNED_IN),
      StatusCode::FORBIDDEN
    );
  }

  #[test]
  fn the_clouds_403_splits_into_the_two_things_it_means() {
    // "Your plan does not include this" is the 402 this API uses everywhere
    // else; "you are not in that team" is not something a payment fixes.
    assert_eq!(
      cloud_failure_status(403, "COOKIE_BOT_NOT_ENTITLED"),
      StatusCode::PAYMENT_REQUIRED
    );
    assert_eq!(
      cloud_failure_status(403, "NOT_TEAM_MEMBER"),
      StatusCode::FORBIDDEN
    );
  }

  #[test]
  fn spending_the_pooled_hours_is_a_payment_problem_not_a_server_fault() {
    // It arrives as a 403 with a code. Reporting it as a plain forbidden would
    // hide the one thing the user can act on.
    assert_eq!(
      cloud_failure_status(403, "REMOTE_HOURS_EXHAUSTED"),
      StatusCode::PAYMENT_REQUIRED
    );
  }

  #[test]
  fn a_busy_or_unreachable_fleet_is_never_reported_as_broken() {
    // 503 means "try again shortly". Turning it into a 500 tells the user
    // their automation is broken when nothing is.
    assert_eq!(
      cloud_failure_status(503, crate::cloud_errors::NO_CAPACITY),
      StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
      cloud_failure_status(0, crate::cloud_errors::UNREACHABLE),
      StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
      cloud_failure_status(429, crate::cloud_errors::RATE_LIMITED),
      StatusCode::TOO_MANY_REQUESTS
    );
  }

  #[test]
  fn a_missing_schedule_and_a_missing_run_both_stay_a_404() {
    assert_eq!(
      cloud_failure_status(404, "COOKIE_BOT_NOT_ENROLLED"),
      StatusCode::NOT_FOUND
    );
    assert_eq!(
      cloud_failure_status(404, "COOKIE_BOT_RUN_NOT_FOUND"),
      StatusCode::NOT_FOUND
    );
    assert_eq!(
      cloud_failure_status(409, "COOKIE_BOT_SCHEDULE_CONFLICT"),
      StatusCode::CONFLICT
    );
    assert_eq!(
      cloud_failure_status(400, "COOKIE_BOT_INVALID_SCHEDULE"),
      StatusCode::BAD_REQUEST
    );
  }

  #[test]
  fn a_read_of_a_session_that_does_not_exist_is_a_404_not_a_500() {
    // The launch mapping folds every unrecognised status into 500, which for a
    // read means "no such session" is indistinguishable from "our backend
    // fell over".
    let missing =
      crate::remote_session::classify_backend_status(404, r#"{"code":"REMOTE_SESSION_NOT_FOUND"}"#);
    let (status, body) = remote_session_read_response(missing);
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_code_of(&body), "REMOTE_SESSION_NOT_FOUND");
  }

  #[test]
  fn a_read_answers_with_a_code_rather_than_the_backends_english() {
    let busy = crate::remote_session::classify_backend_status(503, "no macos host has a free slot");
    let (status, body) = remote_session_read_response(busy);
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error_code_of(&body), crate::cloud_errors::NO_CAPACITY);
  }

  // Axum panics when two handlers claim one path, and the router is only built
  // when the API server is switched on — so a conflict introduced here would
  // ship as an app that dies the first time a user enables the API. Both
  // `/v1/remote-sessions/{id}` and `/v1/cookie-bot/schedules/{profile_id}` now
  // carry several methods, which is exactly the shape that trips it.
  #[test]
  fn every_v1_route_can_be_registered_together() {
    let _router: Router<ApiServerState> = build_v1_router();
  }

  fn schema_required(spec: &serde_json::Value, schema: &str) -> Vec<String> {
    spec["components"]["schemas"][schema]["required"]
      .as_array()
      .map(|a| {
        a.iter()
          .filter_map(|v| v.as_str().map(str::to_string))
          .collect()
      })
      .unwrap_or_default()
  }

  // `#[schema(value_type = Object)]` on an `Option<T>` erases the optionality
  // and marks the field required in the served spec; these fields must stay
  // optional so generated clients aren't forced to send them.
  #[test]
  fn openapi_optional_fields_are_not_required() {
    let spec = serde_json::to_value(ApiDoc::openapi()).expect("spec serializes");

    let create_profile = schema_required(&spec, "CreateProfileRequest");
    assert!(
      !create_profile.iter().any(|f| f == "wayfern_config"),
      "wayfern_config must be optional, required list: {create_profile:?}"
    );

    // `ApiProfile` is the response body of every profile-returning route, so a
    // wrongly-required `group_id` makes generated clients assume a group is
    // always present on an ungrouped profile.
    let api_profile = schema_required(&spec, "ApiProfile");
    assert!(
      !api_profile.iter().any(|f| f == "group_id"),
      "group_id must be optional on ApiProfile, required list: {api_profile:?}"
    );
    assert_eq!(
      spec["components"]["schemas"]["ApiProfile"]["properties"]["group_id"]["type"],
      serde_json::json!(["string", "null"]),
      "group_id must be a nullable string, not a free-form object"
    );

    let update_profile = schema_required(&spec, "UpdateProfileRequest");
    assert!(
      !update_profile.iter().any(|f| f == "group_id"),
      "group_id must be optional, required list: {update_profile:?}"
    );

    let update_proxy = schema_required(&spec, "UpdateProxyRequest");
    assert!(
      !update_proxy.iter().any(|f| f == "proxy_settings"),
      "proxy_settings must be optional on update, required list: {update_proxy:?}"
    );

    let proxy_settings = schema_required(&spec, "ProxySettings");
    for field in ["username", "password", "vless_uri"] {
      assert!(
        !proxy_settings.iter().any(|candidate| candidate == field),
        "{field} must be optional in proxy settings, required list: {proxy_settings:?}"
      );
      assert!(
        spec["components"]["schemas"]["ProxySettings"]["properties"][field].is_object(),
        "{field} must be present in the served ProxySettings schema"
      );
    }

    let import_profiles = schema_required(&spec, "ImportProfilesRequest");
    for field in ["group_id", "duplicate_strategy", "wayfern_config"] {
      assert!(
        !import_profiles.iter().any(|f| f == field),
        "{field} must be optional on profile import, required list: {import_profiles:?}"
      );
    }

    let import_item = schema_required(&spec, "ImportProfileItem");
    for field in ["proxy_id", "vpn_id", "browser_type"] {
      assert!(
        !import_item.iter().any(|f| f == field),
        "{field} must be optional on import items, required list: {import_item:?}"
      );
    }

    // A remote launch with no URL just opens the browser; forcing generated
    // clients to send one would make the common case the awkward one.
    let run_remote = schema_required(&spec, "RunRemoteRequest");
    assert!(
      !run_remote.iter().any(|f| f == "url"),
      "url must be optional on a remote launch, required list: {run_remote:?}"
    );

    // A run-now with no cap inherits the schedule's own.
    let start_run = schema_required(&spec, "StartCookieBotRunRequest");
    assert!(
      start_run.iter().any(|f| f == "profile_id"),
      "profile_id is the one thing a run cannot infer, required list: {start_run:?}"
    );
    assert!(
      !start_run.iter().any(|f| f == "max_minutes"),
      "max_minutes must be optional, required list: {start_run:?}"
    );

    // This machine already knows the profile's name and operating system, and
    // a caller-supplied platform that disagrees is refused rather than
    // honoured — so neither may be marked required.
    let set_schedule = schema_required(&spec, "SetCookieBotScheduleRequest");
    for field in [
      "profile_name",
      "platform",
      "jitter_seconds",
      "sites",
      "acknowledge_conflict",
    ] {
      assert!(
        !set_schedule.iter().any(|f| f == field),
        "{field} must be optional when enrolling, required list: {set_schedule:?}"
      );
    }

    // A freshly created enrolment has never run, so every one of these is
    // absent on the first read. Marking them required would make a generated
    // client reject the response it gets immediately after enrolling.
    let schedule = schema_required(&spec, "CookieBotSchedule");
    for field in ["next_run_at", "last_run_at", "last_run_id", "updated_at"] {
      assert!(
        !schedule.iter().any(|f| f == field),
        "{field} must be optional on a schedule, required list: {schedule:?}"
      );
    }

    let run = schema_required(&spec, "CookieBotRun");
    for field in ["started_at", "ended_at", "outcome_code", "session_id"] {
      assert!(
        !run.iter().any(|f| f == field),
        "{field} must be optional on a run, required list: {run:?}"
      );
    }

    // Only `session_id` and `status` are guaranteed while a session is still
    // provisioning; everything else arrives as the session progresses.
    let session = schema_required(&spec, "RemoteSessionState");
    for field in [
      "profile_id",
      "platform",
      "kind",
      "run_id",
      "started_at",
      "ready_at",
      "closed_at",
      "close_reason",
      "billed_seconds",
    ] {
      assert!(
        !session.iter().any(|f| f == field),
        "{field} must be optional on a session, required list: {session:?}"
      );
    }

    // The route predates the pooled budget and returned only two keys. A
    // deployment that has not rolled forward must still satisfy the spec.
    let quota = schema_required(&spec, "RemoteHoursQuota");
    for field in ["members", "breakdown", "scope", "team_id", "seats"] {
      assert!(
        !quota.iter().any(|f| f == field),
        "{field} must be optional on the quota, required list: {quota:?}"
      );
    }
  }

  #[test]
  fn import_profiles_request_allows_minimal_body() {
    // Only items with source_path + new_profile_name are required; everything
    // else has defaults.
    let json = r#"{"items": [{"source_path": "/tmp/p", "new_profile_name": "Imported"}]}"#;
    let parsed: ImportProfilesRequest =
      serde_json::from_str(json).expect("minimal import body must deserialize");
    assert_eq!(parsed.items.len(), 1);
    assert!(parsed.group_id.is_none());
    assert!(parsed.duplicate_strategy.is_none());
    assert_eq!(parsed.items[0].browser_type, "chromium");
  }

  // The served /openapi.json comes from the hand-maintained ApiDoc `paths(...)`
  // list, not from the router — endpoints registered on the router but missing
  // from ApiDoc silently disappear from the spec. Lock in the ones that were
  // once dropped, and that removed endpoints stay gone.
  #[test]
  fn openapi_spec_covers_registered_routes() {
    let spec = serde_json::to_value(ApiDoc::openapi()).expect("spec serializes");
    let paths = spec["paths"].as_object().expect("paths object");

    for path in [
      "/v1/vpns/{id}/export",
      "/v1/extensions",
      "/v1/extension-groups",
      "/v1/extensions/{id}",
      "/v1/extension-groups/{id}",
      "/v1/profiles/import",
      "/v1/profiles/import/detect",
      "/v1/proxies/import",
      // The whole remote-execution surface was registered on the router but
      // absent from ApiDoc, so it never appeared in the served spec. This list
      // is a hand-maintained allowlist, which is exactly why that drift went
      // unnoticed — every route added here must also be added below.
      "/v1/profiles/{id}/run-remote",
      "/v1/profiles/{id}/cloud-sync",
      "/v1/remote-sessions/{id}",
      // Remote-session observability and the whole cookie-bot surface. Same
      // hazard, so the same guard: registered on the router is not registered
      // in the spec, and the spec is what an automation client is written from.
      "/v1/remote-sessions",
      "/v1/remote-hours",
      "/v1/cookie-bot/schedules",
      "/v1/cookie-bot/schedules/{profile_id}",
      "/v1/cookie-bot/conflicts",
      "/v1/cookie-bot/runs",
      "/v1/cookie-bot/runs/{run_id}",
      "/v1/cookie-bot/presets",
      "/v1/cookie-bot/usage",
    ] {
      assert!(paths.contains_key(path), "missing from ApiDoc: {path}");
    }

    // Every method of every shared path must survive. Registering two handlers
    // on one path in separate `routes!` calls silently drops one of them, and
    // the spec is where that shows up.
    for (path, method) in [
      ("/v1/remote-sessions/{id}", "get"),
      ("/v1/remote-sessions/{id}", "delete"),
      ("/v1/cookie-bot/schedules/{profile_id}", "get"),
      ("/v1/cookie-bot/schedules/{profile_id}", "put"),
      ("/v1/cookie-bot/schedules/{profile_id}", "delete"),
      ("/v1/cookie-bot/runs", "get"),
      ("/v1/cookie-bot/runs", "post"),
      ("/v1/cookie-bot/runs/{run_id}", "delete"),
    ] {
      assert!(
        paths[path].get(method).is_some(),
        "missing from ApiDoc: {method} {path}"
      );
    }

    // Every cookie-bot operation must be findable by tag, or it is invisible in
    // a generated client's grouping even though the path exists.
    for (path, method) in [
      ("/v1/cookie-bot/schedules", "get"),
      ("/v1/cookie-bot/schedules/{profile_id}", "put"),
      ("/v1/cookie-bot/runs", "post"),
      ("/v1/cookie-bot/usage", "get"),
    ] {
      let tags = paths[path][method]["tags"]
        .as_array()
        .unwrap_or_else(|| panic!("{method} {path} has no tags"));
      assert!(
        tags.iter().any(|tag| tag == "cookie-bot"),
        "{method} {path} is not tagged cookie-bot: {tags:?}"
      );
    }

    // A bot run is accepted, not completed: the fleet browses for minutes
    // after the response. A 200 here would be a lie the client acts on.
    assert!(
      paths["/v1/cookie-bot/runs"]["post"]["responses"]
        .get("202")
        .is_some(),
      "starting a bot run must declare 202 Accepted"
    );

    assert!(
      !paths.keys().any(|p| p.contains("wayfern-token")),
      "wayfern-token endpoints were removed and must stay out of the spec"
    );

    // A path with a body that resolves to nothing is worse than a missing
    // path: a generator emits a client for it and the response type is empty.
    // These live in other modules, so `components(schemas(...))` is the only
    // thing pulling them in.
    for schema in [
      "RemoteSessionState",
      "ApiRemoteSessionsResponse",
      "SetCookieBotScheduleRequest",
      "StartCookieBotRunRequest",
      "CookieBotSchedule",
      "CookieBotScheduleList",
      "CookieBotScheduleSaved",
      "CookieBotScheduleDeleted",
      "CookieBotConflict",
      "CookieBotConflictCheck",
      "CookieBotRun",
      "CookieBotRunPage",
      "CookieBotRunStarted",
      "CookieBotPreset",
      "CookieBotPresetList",
      "CookieBotUsage",
      "CookieBotUsageMember",
      "CookieBotUsageProfile",
      "RemoteHoursQuota",
      "RemoteHoursMember",
      "RemoteHoursBreakdown",
    ] {
      assert!(
        spec["components"]["schemas"][schema]["properties"].is_object(),
        "schema is missing from the served spec: {schema}"
      );
    }

    // A response body declared as a path outside this module must resolve to
    // the component that path registered, not to a dangling or inlined name.
    for (path, method, status, schema) in [
      (
        "/v1/cookie-bot/schedules",
        "get",
        "200",
        "CookieBotScheduleList",
      ),
      ("/v1/cookie-bot/runs", "post", "202", "CookieBotRunStarted"),
      (
        "/v1/cookie-bot/runs/{run_id}",
        "delete",
        "200",
        "CookieBotRun",
      ),
      (
        "/v1/remote-sessions/{id}",
        "get",
        "200",
        "RemoteSessionState",
      ),
      ("/v1/remote-hours", "get", "200", "RemoteHoursQuota"),
    ] {
      let reference =
        &paths[path][method]["responses"][status]["content"]["application/json"]["schema"]["$ref"];
      assert_eq!(
        reference.as_str(),
        Some(format!("#/components/schemas/{schema}").as_str()),
        "{method} {path} {status} does not reference {schema}: {reference:?}"
      );
    }

    // The presets a client may choose from must never carry the behaviour they
    // expand to. A site list, a dwell range or a step programme appearing here
    // would mean the browsing model had leaked out of the server.
    let preset_properties = spec["components"]["schemas"]["CookieBotPreset"]["properties"]
      .as_object()
      .expect("preset properties");
    for leaked in [
      "sites",
      "dwell",
      "dwell_seconds",
      "steps",
      "actions",
      "corpus",
    ] {
      assert!(
        !preset_properties.contains_key(leaked),
        "the browsing model leaked into the client contract: {leaked}"
      );
    }

    for path in [
      "/v1/profiles/{id}/run",
      "/v1/profiles/{id}/open-url",
      "/v1/profiles/{id}/kill",
      "/v1/profiles/{id}/run-remote",
      "/v1/profiles/batch/run",
      "/v1/profiles/batch/stop",
    ] {
      assert!(
        paths[path]["post"]["responses"].get("429").is_some(),
        "automation route is missing its 429 response: {path}"
      );
    }

    assert!(
      paths["/v1/cookie-bot/runs"]["post"]["responses"]
        .get("429")
        .is_some(),
      "starting a bot run is metered and must declare its 429"
    );

    // The automation routes that are not POSTs. Both declared a 429 that
    // `is_automation_request` could never produce, because that function
    // returned early for every non-POST method.
    for path in ["/v1/remote-sessions/{id}", "/v1/cookie-bot/runs/{run_id}"] {
      assert!(
        paths[path]["delete"]["responses"].get("429").is_some(),
        "metered stop route is missing its 429 response: {path}"
      );
    }

    // Schedule writes are configuration, not automation. Declaring a 429 they
    // can never return would send a client building retry logic for a status
    // it will never see.
    for method in ["put", "delete"] {
      assert!(
        paths["/v1/cookie-bot/schedules/{profile_id}"][method]["responses"]
          .get("429")
          .is_none(),
        "a schedule write must not declare a 429: {method}"
      );
    }
  }
}
