use crate::camoufox_manager::CamoufoxConfig;
use crate::group_manager::GROUP_MANAGER;
use crate::profile::manager::ProfileManager;
use crate::proxy_manager::PROXY_MANAGER;
use crate::tag_manager::TAG_MANAGER;
use axum::{
  extract::{Path, Query, State},
  http::{HeaderMap, StatusCode},
  middleware::{self, Next},
  response::{Json, Response},
  routing::{delete, get, post, put},
  Router,
};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::Emitter;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use tower_http::cors::CorsLayer;

// API Types
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiProfile {
  pub id: String,
  pub name: String,
  pub browser: String,
  pub version: String,
  pub proxy_id: Option<String>,
  pub process_id: Option<u32>,
  pub last_launch: Option<u64>,
  pub release_type: String,
  pub camoufox_config: Option<serde_json::Value>,
  pub group_id: Option<String>,
  pub tags: Vec<String>,
  pub is_running: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiProfilesResponse {
  pub profiles: Vec<ApiProfile>,
  pub total: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiProfileResponse {
  pub profile: ApiProfile,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateProfileRequest {
  pub name: String,
  pub browser: String,
  pub version: String,
  pub proxy_id: Option<String>,
  pub release_type: Option<String>,
  pub camoufox_config: Option<serde_json::Value>,
  pub group_id: Option<String>,
  pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateProfileRequest {
  pub name: Option<String>,
  pub browser: Option<String>,
  pub version: Option<String>,
  pub proxy_id: Option<String>,
  pub release_type: Option<String>,
  pub camoufox_config: Option<serde_json::Value>,
  pub group_id: Option<String>,
  pub tags: Option<Vec<String>>,
}

#[derive(Clone)]
struct ApiServerState {
  app_handle: tauri::AppHandle,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiGroupResponse {
  id: String,
  name: String,
  profile_count: usize,
}

#[derive(Debug, Deserialize)]
struct CreateGroupRequest {
  name: String,
}

#[derive(Debug, Deserialize)]
struct UpdateGroupRequest {
  name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiProxyResponse {
  id: String,
  name: String,
  proxy_settings: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct CreateProxyRequest {
  name: String,
  proxy_settings: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct UpdateProxyRequest {
  name: Option<String>,
  proxy_settings: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct DownloadBrowserRequest {
  browser: String,
  version: String,
}

#[derive(Debug, Serialize)]
struct DownloadBrowserResponse {
  browser: String,
  version: String,
  status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToastPayload {
  pub message: String,
  pub variant: String,
  pub title: String,
  pub description: Option<String>,
}

#[derive(Debug, Serialize)]
struct RunProfileResponse {
  profile_id: String,
  remote_debugging_port: u16,
  headless: bool,
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
            let _ = app_handle.emit(
              "api-port-conflict",
              format!("API server using fallback port {random_port}"),
            );
            listener
          }
          Err(e) => return Err(format!("Failed to bind to any port: {e}")),
        }
      }
    };

    let actual_port = listener
      .local_addr()
      .map_err(|e| format!("Failed to get local address: {e}"))?
      .port();

    // Create router with CORS, authentication, and versioning
    let v1_routes = Router::new()
      .route("/profiles", get(get_profiles))
      .route("/profiles", post(create_profile))
      .route("/profiles/{id}", get(get_profile))
      .route("/profiles/{id}", put(update_profile))
      .route("/profiles/{id}", delete(delete_profile))
      .route("/profiles/{id}/run", post(run_profile))
      .route("/groups", get(get_groups).post(create_group))
      .route(
        "/groups/{id}",
        get(get_group).put(update_group).delete(delete_group),
      )
      .route("/tags", get(get_tags))
      .route("/proxies", get(get_proxies).post(create_proxy))
      .route(
        "/proxies/{id}",
        get(get_proxy).put(update_proxy).delete(delete_proxy),
      )
      .route("/browsers/download", post(download_browser_api))
      .route("/browsers/{browser}/versions", get(get_browser_versions))
      .route(
        "/browsers/{browser}/versions/{version}/downloaded",
        get(check_browser_downloaded),
      )
      .layer(middleware::from_fn_with_state(
        state.clone(),
        auth_middleware,
      ));

    let app = Router::new()
      .nest("/v1", v1_routes)
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

// Authentication middleware
async fn auth_middleware(
  State(state): State<ApiServerState>,
  headers: HeaderMap,
  request: axum::extract::Request,
  next: Next,
) -> Result<Response, StatusCode> {
  // Get the Authorization header
  let auth_header = headers
    .get("Authorization")
    .and_then(|h| h.to_str().ok())
    .and_then(|h| h.strip_prefix("Bearer "));

  let token = match auth_header {
    Some(token) => token,
    None => return Err(StatusCode::UNAUTHORIZED),
  };

  // Get the stored token
  let settings_manager = crate::settings_manager::SettingsManager::instance();
  let stored_token = match settings_manager.get_api_token(&state.app_handle).await {
    Ok(Some(stored_token)) => stored_token,
    Ok(None) => return Err(StatusCode::UNAUTHORIZED),
    Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
  };

  // Compare tokens
  if token != stored_token {
    return Err(StatusCode::UNAUTHORIZED);
  }

  // Token is valid, continue with the request
  Ok(next.run(request).await)
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
async fn get_profiles() -> Result<Json<ApiProfilesResponse>, StatusCode> {
  let profile_manager = ProfileManager::instance();
  match profile_manager.list_profiles() {
    Ok(profiles) => {
      let api_profiles: Vec<ApiProfile> = profiles
        .iter()
        .map(|profile| ApiProfile {
          id: profile.id.to_string(),
          name: profile.name.clone(),
          browser: profile.browser.clone(),
          version: profile.version.clone(),
          proxy_id: profile.proxy_id.clone(),
          process_id: profile.process_id,
          last_launch: profile.last_launch,
          release_type: profile.release_type.clone(),
          camoufox_config: profile
            .camoufox_config
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
          group_id: profile.group_id.clone(),
          tags: profile.tags.clone(),
          is_running: profile.process_id.is_some(), // Simple check based on process_id
        })
        .collect();

      Ok(Json(ApiProfilesResponse {
        profiles: api_profiles,
        total: profiles.len(),
      }))
    }
    Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
  }
}

async fn get_profile(
  Path(id): Path<String>,
  State(_state): State<ApiServerState>,
) -> Result<Json<ApiProfileResponse>, StatusCode> {
  let profile_manager = ProfileManager::instance();
  match profile_manager.list_profiles() {
    Ok(profiles) => {
      if let Some(profile) = profiles.iter().find(|p| p.id.to_string() == id) {
        Ok(Json(ApiProfileResponse {
          profile: ApiProfile {
            id: profile.id.to_string(),
            name: profile.name.clone(),
            browser: profile.browser.clone(),
            version: profile.version.clone(),
            proxy_id: profile.proxy_id.clone(),
            process_id: profile.process_id,
            last_launch: profile.last_launch,
            release_type: profile.release_type.clone(),
            camoufox_config: profile
              .camoufox_config
              .as_ref()
              .and_then(|c| serde_json::to_value(c).ok()),
            group_id: profile.group_id.clone(),
            tags: profile.tags.clone(),
            is_running: profile.process_id.is_some(), // Simple check based on process_id
          },
        }))
      } else {
        Err(StatusCode::NOT_FOUND)
      }
    }
    Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
  }
}

async fn create_profile(
  State(state): State<ApiServerState>,
  Json(request): Json<CreateProfileRequest>,
) -> Result<Json<ApiProfileResponse>, StatusCode> {
  let profile_manager = ProfileManager::instance();

  // Parse camoufox config if provided
  let camoufox_config = if let Some(config) = &request.camoufox_config {
    serde_json::from_value(config.clone()).ok()
  } else {
    None
  };

  // Create profile using the async create_profile_with_group method
  match profile_manager
    .create_profile_with_group(
      &state.app_handle,
      &request.name,
      &request.browser,
      &request.version,
      request.release_type.as_deref().unwrap_or("stable"),
      request.proxy_id.clone(),
      camoufox_config,
      request.group_id.clone(),
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
          return Err(StatusCode::INTERNAL_SERVER_ERROR);
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
        profile: ApiProfile {
          id: profile.id.to_string(),
          name: profile.name,
          browser: profile.browser,
          version: profile.version,
          proxy_id: profile.proxy_id,
          process_id: profile.process_id,
          last_launch: profile.last_launch,
          release_type: profile.release_type,
          camoufox_config: profile
            .camoufox_config
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
          group_id: profile.group_id,
          tags: profile.tags,
          is_running: false,
        },
      }))
    }
    Err(_) => Err(StatusCode::BAD_REQUEST),
  }
}

async fn update_profile(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
  Json(request): Json<UpdateProfileRequest>,
) -> Result<Json<ApiProfileResponse>, StatusCode> {
  let profile_manager = ProfileManager::instance();

  // Update profile fields
  if let Some(new_name) = request.name {
    if profile_manager
      .rename_profile(&state.app_handle, &id, &new_name)
      .is_err()
    {
      return Err(StatusCode::BAD_REQUEST);
    }
  }

  if let Some(version) = request.version {
    if profile_manager
      .update_profile_version(&state.app_handle, &id, &version)
      .is_err()
    {
      return Err(StatusCode::BAD_REQUEST);
    }
  }

  if let Some(proxy_id) = request.proxy_id {
    if profile_manager
      .update_profile_proxy(state.app_handle.clone(), &id, Some(proxy_id))
      .await
      .is_err()
    {
      return Err(StatusCode::BAD_REQUEST);
    }
  }

  if let Some(camoufox_config) = request.camoufox_config {
    let config: Result<CamoufoxConfig, _> = serde_json::from_value(camoufox_config);
    match config {
      Ok(config) => {
        if profile_manager
          .update_camoufox_config(state.app_handle.clone(), &id, config)
          .await
          .is_err()
        {
          return Err(StatusCode::BAD_REQUEST);
        }
      }
      Err(_) => return Err(StatusCode::BAD_REQUEST),
    }
  }

  if let Some(group_id) = request.group_id {
    if profile_manager
      .assign_profiles_to_group(&state.app_handle, vec![id.clone()], Some(group_id))
      .is_err()
    {
      return Err(StatusCode::BAD_REQUEST);
    }
  }

  if let Some(tags) = request.tags {
    if profile_manager
      .update_profile_tags(&state.app_handle, &id, tags)
      .is_err()
    {
      return Err(StatusCode::BAD_REQUEST);
    }

    // Update tag manager with new tags from all profiles
    if let Ok(profiles) = profile_manager.list_profiles() {
      let _ = crate::tag_manager::TAG_MANAGER
        .lock()
        .map(|manager| manager.rebuild_from_profiles(&profiles));
    }
  }

  // Return updated profile
  get_profile(Path(id), State(state)).await
}

async fn delete_profile(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
) -> Result<StatusCode, StatusCode> {
  let profile_manager = ProfileManager::instance();
  match profile_manager.delete_profile(&state.app_handle, &id) {
    Ok(_) => Ok(StatusCode::NO_CONTENT),
    Err(_) => Err(StatusCode::BAD_REQUEST),
  }
}

// API Handlers - Groups
async fn get_groups(
  State(_state): State<ApiServerState>,
) -> Result<Json<Vec<ApiGroupResponse>>, StatusCode> {
  match GROUP_MANAGER.lock() {
    Ok(manager) => {
      match manager.get_all_groups() {
        Ok(groups) => {
          let api_groups = groups
            .into_iter()
            .map(|group| ApiGroupResponse {
              id: group.id,
              name: group.name,
              profile_count: 0, // Would need profile list to calculate this
            })
            .collect();
          Ok(Json(api_groups))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
      }
    }
    Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
  }
}

async fn get_group(
  Path(id): Path<String>,
  State(_state): State<ApiServerState>,
) -> Result<Json<ApiGroupResponse>, StatusCode> {
  match GROUP_MANAGER.lock() {
    Ok(manager) => match manager.get_all_groups() {
      Ok(groups) => {
        if let Some(group) = groups.into_iter().find(|g| g.id == id) {
          Ok(Json(ApiGroupResponse {
            id: group.id,
            name: group.name,
            profile_count: 0,
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

async fn create_group(
  State(state): State<ApiServerState>,
  Json(request): Json<CreateGroupRequest>,
) -> Result<Json<ApiGroupResponse>, StatusCode> {
  match GROUP_MANAGER.lock() {
    Ok(manager) => match manager.create_group(&state.app_handle, request.name) {
      Ok(group) => Ok(Json(ApiGroupResponse {
        id: group.id,
        name: group.name,
        profile_count: 0,
      })),
      Err(_) => Err(StatusCode::BAD_REQUEST),
    },
    Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
  }
}

async fn update_group(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
  Json(request): Json<UpdateGroupRequest>,
) -> Result<Json<ApiGroupResponse>, StatusCode> {
  match GROUP_MANAGER.lock() {
    Ok(manager) => match manager.update_group(&state.app_handle, id.clone(), request.name) {
      Ok(group) => Ok(Json(ApiGroupResponse {
        id: group.id,
        name: group.name,
        profile_count: 0,
      })),
      Err(_) => Err(StatusCode::BAD_REQUEST),
    },
    Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
  }
}

async fn delete_group(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
) -> Result<StatusCode, StatusCode> {
  match GROUP_MANAGER.lock() {
    Ok(manager) => match manager.delete_group(&state.app_handle, id.clone()) {
      Ok(_) => Ok(StatusCode::NO_CONTENT),
      Err(_) => Err(StatusCode::BAD_REQUEST),
    },
    Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
  }
}

// API Handlers - Tags
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
        proxy_settings: serde_json::to_value(p.proxy_settings).unwrap_or_default(),
      })
      .collect(),
  ))
}

async fn get_proxy(
  Path(id): Path<String>,
  State(_state): State<ApiServerState>,
) -> Result<Json<ApiProxyResponse>, StatusCode> {
  let proxies = PROXY_MANAGER.get_stored_proxies();
  if let Some(proxy) = proxies.into_iter().find(|p| p.id == id) {
    Ok(Json(ApiProxyResponse {
      id: proxy.id,
      name: proxy.name,
      proxy_settings: serde_json::to_value(proxy.proxy_settings).unwrap_or_default(),
    }))
  } else {
    Err(StatusCode::NOT_FOUND)
  }
}

async fn create_proxy(
  State(state): State<ApiServerState>,
  Json(request): Json<CreateProxyRequest>,
) -> Result<Json<ApiProxyResponse>, StatusCode> {
  // Convert JSON value to ProxySettings
  match serde_json::from_value(request.proxy_settings.clone()) {
    Ok(proxy_settings) => {
      match PROXY_MANAGER.create_stored_proxy(
        &state.app_handle,
        request.name.clone(),
        proxy_settings,
      ) {
        Ok(_) => {
          // Find the created proxy to return it
          let proxies = PROXY_MANAGER.get_stored_proxies();
          if let Some(proxy) = proxies.into_iter().find(|p| p.name == request.name) {
            Ok(Json(ApiProxyResponse {
              id: proxy.id,
              name: proxy.name,
              proxy_settings: request.proxy_settings,
            }))
          } else {
            Err(StatusCode::INTERNAL_SERVER_ERROR)
          }
        }
        Err(_) => Err(StatusCode::BAD_REQUEST),
      }
    }
    Err(_) => Err(StatusCode::BAD_REQUEST),
  }
}

async fn update_proxy(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
  Json(request): Json<UpdateProxyRequest>,
) -> Result<Json<ApiProxyResponse>, StatusCode> {
  let proxies = PROXY_MANAGER.get_stored_proxies();
  if let Some(proxy) = proxies.into_iter().find(|p| p.id == id) {
    let new_name = request.name.unwrap_or(proxy.name.clone());
    let new_proxy_settings = if let Some(settings_json) = request.proxy_settings {
      match serde_json::from_value(settings_json) {
        Ok(settings) => settings,
        Err(_) => return Err(StatusCode::BAD_REQUEST),
      }
    } else {
      proxy.proxy_settings.clone()
    };

    match PROXY_MANAGER.update_stored_proxy(
      &state.app_handle,
      &id,
      Some(new_name.clone()),
      Some(new_proxy_settings.clone()),
    ) {
      Ok(_) => Ok(Json(ApiProxyResponse {
        id,
        name: new_name,
        proxy_settings: serde_json::to_value(new_proxy_settings).unwrap_or_default(),
      })),
      Err(_) => Err(StatusCode::BAD_REQUEST),
    }
  } else {
    Err(StatusCode::NOT_FOUND)
  }
}

async fn delete_proxy(
  Path(id): Path<String>,
  State(state): State<ApiServerState>,
) -> Result<StatusCode, StatusCode> {
  match PROXY_MANAGER.delete_stored_proxy(&state.app_handle, &id) {
    Ok(_) => Ok(StatusCode::NO_CONTENT),
    Err(_) => Err(StatusCode::BAD_REQUEST),
  }
}

// API Handler - Run Profile with Remote Debugging
async fn run_profile(
  Path(id): Path<String>,
  Query(params): Query<HashMap<String, String>>,
  State(state): State<ApiServerState>,
) -> Result<Json<RunProfileResponse>, StatusCode> {
  let headless = params
    .get("headless")
    .and_then(|v| v.parse::<bool>().ok())
    .unwrap_or(false);

  let profile_manager = ProfileManager::instance();
  let profiles = profile_manager
    .list_profiles()
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

  let profile = profiles
    .iter()
    .find(|p| p.id.to_string() == id)
    .ok_or(StatusCode::NOT_FOUND)?;

  // Generate a random port for remote debugging
  let remote_debugging_port = rand::random::<u16>().saturating_add(9000).max(9000);

  // Use the same launch method as the main app, but with remote debugging enabled
  match crate::browser_runner::launch_browser_profile_with_debugging(
    state.app_handle.clone(),
    profile.clone(),
    None,
    Some(remote_debugging_port),
    headless,
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

// API Handler - Download Browser
async fn download_browser_api(
  State(state): State<ApiServerState>,
  Json(request): Json<DownloadBrowserRequest>,
) -> Result<Json<DownloadBrowserResponse>, StatusCode> {
  match crate::downloader::download_browser(
    state.app_handle.clone(),
    request.browser.clone(),
    request.version.clone(),
  )
  .await
  {
    Ok(_) => Ok(Json(DownloadBrowserResponse {
      browser: request.browser,
      version: request.version,
      status: "downloaded".to_string(),
    })),
    Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
  }
}

// API Handler - Get Browser Versions
async fn get_browser_versions(
  Path(browser): Path<String>,
  State(_state): State<ApiServerState>,
) -> Result<Json<Vec<String>>, StatusCode> {
  let version_manager = crate::browser_version_manager::BrowserVersionManager::instance();

  match version_manager
    .fetch_browser_versions_with_count(&browser, false)
    .await
  {
    Ok(result) => Ok(Json(result.versions)),
    Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
  }
}

// API Handler - Check if Browser is Downloaded
async fn check_browser_downloaded(
  Path((browser, version)): Path<(String, String)>,
  State(_state): State<ApiServerState>,
) -> Result<Json<bool>, StatusCode> {
  let is_downloaded = crate::downloaded_browsers_registry::is_browser_downloaded(browser, version);
  Ok(Json(is_downloaded))
}
