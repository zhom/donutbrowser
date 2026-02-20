use crate::browser::ProxySettings;
use crate::camoufox_manager::CamoufoxConfig;
use crate::daemon_ws::{ws_handler, WsState};
use crate::events;
use crate::group_manager::GROUP_MANAGER;
use crate::profile::manager::ProfileManager;
use crate::proxy_manager::PROXY_MANAGER;
use crate::tag_manager::TAG_MANAGER;
use axum::{
  extract::{Path, State},
  http::{HeaderMap, StatusCode},
  middleware::{self, Next},
  response::{Json, Response},
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
  pub process_id: Option<u32>,
  pub last_launch: Option<u64>,
  pub release_type: String,
  #[schema(value_type = Object)]
  pub camoufox_config: Option<serde_json::Value>,
  pub group_id: Option<String>,
  pub tags: Vec<String>,
  pub is_running: bool,
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
  pub browser: String,
  pub version: String,
  pub proxy_id: Option<String>,
  pub release_type: Option<String>,
  #[schema(value_type = Object)]
  pub camoufox_config: Option<serde_json::Value>,
  #[schema(value_type = Object)]
  pub wayfern_config: Option<serde_json::Value>,
  pub group_id: Option<String>,
  pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateProfileRequest {
  pub name: Option<String>,
  pub browser: Option<String>,
  pub version: Option<String>,
  pub proxy_id: Option<String>,
  pub release_type: Option<String>,
  #[schema(value_type = Object)]
  pub camoufox_config: Option<serde_json::Value>,
  pub group_id: Option<String>,
  pub tags: Option<Vec<String>>,
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
  #[schema(value_type = Object)]
  proxy_settings: Option<ProxySettings>,
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

#[derive(Debug, Deserialize, ToSchema)]
struct RunProfileRequest {
  url: Option<String>,
  headless: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct OpenUrlRequest {
  url: String,
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
    open_url_in_profile,
    kill_profile,
    get_groups,
    get_group,
    create_group,
    update_group,
    delete_group,
    get_tags,
    get_proxies,
    get_proxy,
    create_proxy,
    update_proxy,
    delete_proxy,
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
    DownloadBrowserRequest,
    DownloadBrowserResponse,
    RunProfileResponse,
    RunProfileRequest,
    OpenUrlRequest,
    ProxySettings,
  )),
  tags(
    (name = "profiles", description = "Profile management endpoints"),
    (name = "groups", description = "Group management endpoints"),
    (name = "tags", description = "Tag management endpoints"),
    (name = "proxies", description = "Proxy management endpoints"),
    (name = "browsers", description = "Browser management endpoints"),
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
          Err(e) => return Err(format!("Failed to bind to any port: {e}")),
        }
      }
    };

    let actual_port = listener
      .local_addr()
      .map_err(|e| format!("Failed to get local address: {e}"))?
      .port();

    // Create router with OpenAPI documentation
    let (v1_routes, _) = OpenApiRouter::new()
      .routes(routes!(
        get_profiles,
        create_profile,
        get_profile,
        update_profile,
        delete_profile,
        run_profile,
        open_url_in_profile,
        kill_profile,
        get_groups,
        create_group,
        get_group,
        update_group,
        delete_group,
        get_tags,
        get_proxies,
        create_proxy,
        get_proxy,
        update_proxy,
        delete_proxy,
        download_browser_api,
        get_browser_versions,
        check_browser_downloaded,
      ))
      .split_for_parts();

    let api = ApiDoc::openapi();

    let v1_routes = v1_routes
      .layer(middleware::from_fn_with_state(
        state.clone(),
        auth_middleware,
      ))
      .layer(middleware::from_fn(terms_check_middleware));

    // Create WebSocket route with its own state (no auth required for daemon IPC)
    let ws_state = WsState::new();
    let ws_routes = Router::new()
      .route("/events", get(ws_handler))
      .with_state(ws_state);

    let app = Router::new()
      .nest("/v1", v1_routes)
      .nest("/ws", ws_routes)
      .route("/openapi.json", get(move || async move { Json(api) }))
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

#[utoipa::path(
  post,
  path = "/v1/profiles",
  request_body = CreateProfileRequest,
  responses(
    (status = 200, description = "Profile created successfully", body = ApiProfileResponse),
    (status = 400, description = "Bad request"),
    (status = 401, description = "Unauthorized"),
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
) -> Result<Json<ApiProfileResponse>, StatusCode> {
  let profile_manager = ProfileManager::instance();

  // Parse camoufox config if provided
  let camoufox_config = if let Some(config) = &request.camoufox_config {
    serde_json::from_value(config.clone()).ok()
  } else {
    None
  };

  // Parse wayfern config if provided
  let wayfern_config = if let Some(config) = &request.wayfern_config {
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
      None, // vpn_id
      camoufox_config,
      wayfern_config,
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
) -> Result<StatusCode, StatusCode> {
  let profile_manager = ProfileManager::instance();
  match profile_manager.delete_profile(&state.app_handle, &id) {
    Ok(_) => Ok(StatusCode::NO_CONTENT),
    Err(_) => Err(StatusCode::BAD_REQUEST),
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

#[utoipa::path(
  delete,
  path = "/v1/groups/{id}",
  params(
    ("id" = String, Path, description = "Group ID")
  ),
  responses(
    (status = 204, description = "Group deleted successfully"),
    (status = 400, description = "Bad request"),
    (status = 401, description = "Unauthorized"),
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
) -> Result<Json<ApiProxyResponse>, StatusCode> {
  match PROXY_MANAGER.create_stored_proxy(
    &state.app_handle,
    request.name.clone(),
    request.proxy_settings,
  ) {
    Ok(proxy) => Ok(Json(ApiProxyResponse {
      id: proxy.id,
      name: proxy.name,
      proxy_settings: proxy.proxy_settings,
    })),
    Err(_) => Err(StatusCode::BAD_REQUEST),
  }
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
) -> Result<Json<ApiProxyResponse>, StatusCode> {
  let proxies = PROXY_MANAGER.get_stored_proxies();
  if let Some(proxy) = proxies.into_iter().find(|p| p.id == id) {
    let new_name = request.name.unwrap_or(proxy.name.clone());
    let new_proxy_settings = request
      .proxy_settings
      .unwrap_or(proxy.proxy_settings.clone());

    match PROXY_MANAGER.update_stored_proxy(
      &state.app_handle,
      &id,
      Some(new_name.clone()),
      Some(new_proxy_settings.clone()),
    ) {
      Ok(_) => Ok(Json(ApiProxyResponse {
        id,
        name: new_name,
        proxy_settings: new_proxy_settings,
      })),
      Err(_) => Err(StatusCode::BAD_REQUEST),
    }
  } else {
    Err(StatusCode::NOT_FOUND)
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
    (status = 400, description = "Bad request"),
    (status = 401, description = "Unauthorized"),
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
) -> Result<StatusCode, StatusCode> {
  match PROXY_MANAGER.delete_stored_proxy(&state.app_handle, &id) {
    Ok(_) => Ok(StatusCode::NO_CONTENT),
    Err(_) => Err(StatusCode::BAD_REQUEST),
  }
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
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "Profile not found"),
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

  // Generate a random port for remote debugging
  let remote_debugging_port = rand::random::<u16>().saturating_add(9000).max(9000);

  // Use the same launch method as the main app, but with remote debugging enabled
  match crate::browser_runner::launch_browser_profile_with_debugging(
    state.app_handle.clone(),
    profile.clone(),
    url,
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
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "Profile not found"),
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
) -> Result<StatusCode, StatusCode> {
  let browser_runner = crate::browser_runner::BrowserRunner::instance();

  browser_runner
    .open_url_with_profile(state.app_handle.clone(), id, request.url)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
    (status = 404, description = "Profile not found"),
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

  Ok(StatusCode::NO_CONTENT)
}

// API Handler - Download Browser
#[utoipa::path(
  post,
  path = "/v1/browsers/download",
  request_body = DownloadBrowserRequest,
  responses(
    (status = 200, description = "Browser download initiated", body = DownloadBrowserResponse),
    (status = 401, description = "Unauthorized"),
    (status = 500, description = "Internal server error")
  ),
  security(
    ("bearer_auth" = [])
  ),
  tag = "browsers"
)]
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
#[utoipa::path(
  get,
  path = "/v1/browsers/{browser}/versions",
  params(
    ("browser" = String, Path, description = "Browser name")
  ),
  responses(
    (status = 200, description = "List of available browser versions", body = Vec<String>),
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
