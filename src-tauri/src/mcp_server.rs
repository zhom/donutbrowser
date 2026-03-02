#![allow(dead_code)]

use axum::{
  body::Body,
  extract::State,
  http::{header, Request, StatusCode},
  middleware::{self, Next},
  response::{IntoResponse, Response},
  routing::post,
  Json, Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use tauri::AppHandle;
use tokio::net::TcpListener;
use tokio::sync::Mutex as AsyncMutex;

use crate::browser::ProxySettings;
use crate::cloud_auth::CLOUD_AUTH;
use crate::group_manager::GROUP_MANAGER;
use crate::profile::{BrowserProfile, ProfileManager};
use crate::proxy_manager::PROXY_MANAGER;
use crate::settings_manager::SettingsManager;
use crate::wayfern_terms::WayfernTermsManager;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
  pub name: String,
  pub description: String,
  pub input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct McpRequest {
  jsonrpc: String,
  id: serde_json::Value,
  method: String,
  params: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct McpResponse {
  jsonrpc: String,
  id: serde_json::Value,
  #[serde(skip_serializing_if = "Option::is_none")]
  result: Option<serde_json::Value>,
  #[serde(skip_serializing_if = "Option::is_none")]
  error: Option<McpError>,
}

#[derive(Debug, Serialize)]
pub struct McpError {
  code: i32,
  message: String,
}

const DEFAULT_MCP_PORT: u16 = 51080;

struct McpServerInner {
  app_handle: Option<AppHandle>,
  token: Option<String>,
  shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

#[derive(Clone)]
struct McpHttpState {
  server: &'static McpServer,
  token: String,
}

pub struct McpServer {
  inner: Arc<AsyncMutex<McpServerInner>>,
  is_running: AtomicBool,
  port: AtomicU16,
}

impl McpServer {
  fn new() -> Self {
    Self {
      inner: Arc::new(AsyncMutex::new(McpServerInner {
        app_handle: None,
        token: None,
        shutdown_tx: None,
      })),
      is_running: AtomicBool::new(false),
      port: AtomicU16::new(0),
    }
  }

  pub fn instance() -> &'static McpServer {
    &MCP_SERVER
  }

  pub fn is_running(&self) -> bool {
    self.is_running.load(Ordering::SeqCst)
  }

  pub fn get_port(&self) -> Option<u16> {
    let port = self.port.load(Ordering::SeqCst);
    if port > 0 {
      Some(port)
    } else {
      None
    }
  }

  pub async fn start(&self, app_handle: AppHandle) -> Result<u16, String> {
    if !WayfernTermsManager::instance().is_terms_accepted() {
      return Err(
        "Wayfern Terms and Conditions must be accepted before starting MCP server".to_string(),
      );
    }

    if self.is_running() {
      return Err("MCP server is already running".to_string());
    }

    let settings_manager = SettingsManager::instance();
    let settings = settings_manager
      .load_settings()
      .map_err(|e| format!("Failed to load settings: {e}"))?;

    // Get or generate token
    let existing_token = settings_manager
      .get_mcp_token(&app_handle)
      .await
      .ok()
      .flatten();

    let token = if let Some(t) = existing_token {
      t
    } else {
      settings_manager
        .generate_mcp_token(&app_handle)
        .await
        .map_err(|e| format!("Failed to generate MCP token: {e}"))?
    };

    // Determine port (use saved port, or try default, or random)
    let preferred_port = settings.mcp_port.unwrap_or(DEFAULT_MCP_PORT);
    let actual_port = self.bind_to_available_port(preferred_port).await?;

    // Save port if it changed
    if settings.mcp_port != Some(actual_port) {
      let mut new_settings = settings;
      new_settings.mcp_port = Some(actual_port);
      settings_manager
        .save_settings(&new_settings)
        .map_err(|e| format!("Failed to save settings: {e}"))?;
    }

    // Store state
    let mut inner = self.inner.lock().await;
    inner.app_handle = Some(app_handle);
    inner.token = Some(token.clone());

    // Create shutdown channel
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    inner.shutdown_tx = Some(shutdown_tx);

    self.port.store(actual_port, Ordering::SeqCst);
    self.is_running.store(true, Ordering::SeqCst);

    // Start HTTP server in background
    let http_state = McpHttpState {
      server: McpServer::instance(),
      token,
    };
    tokio::spawn(Self::run_http_server(actual_port, http_state, shutdown_rx));

    log::info!("[mcp] Server started on port {}", actual_port);
    Ok(actual_port)
  }

  async fn bind_to_available_port(&self, preferred: u16) -> Result<u16, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], preferred));
    if TcpListener::bind(addr).await.is_ok() {
      return Ok(preferred);
    }

    // Try random ports in 51000-51999 range
    for _ in 0..10 {
      let port = 51000 + (rand::random::<u16>() % 1000);
      let addr = SocketAddr::from(([127, 0, 0, 1], port));
      if TcpListener::bind(addr).await.is_ok() {
        return Ok(port);
      }
    }

    Err("Could not find available port for MCP server".to_string())
  }

  async fn run_http_server(
    port: u16,
    state: McpHttpState,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
  ) {
    let app = Router::new()
      .route("/mcp", post(Self::handle_mcp_post))
      .layer(middleware::from_fn_with_state(
        state.clone(),
        Self::auth_middleware,
      ))
      .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = match TcpListener::bind(addr).await {
      Ok(l) => l,
      Err(e) => {
        log::error!("[mcp] Failed to bind to port {}: {}", port, e);
        return;
      }
    };

    log::info!(
      "[mcp] HTTP server listening on http://127.0.0.1:{}/mcp",
      port
    );

    let server = axum::serve(listener, app).with_graceful_shutdown(async {
      let _ = shutdown_rx.await;
      log::info!("[mcp] HTTP server shutting down");
    });

    if let Err(e) = server.await {
      log::error!("[mcp] HTTP server error: {}", e);
    }
  }

  async fn auth_middleware(
    State(state): State<McpHttpState>,
    req: Request<Body>,
    next: Next,
  ) -> Result<Response, StatusCode> {
    let auth_header = req
      .headers()
      .get(header::AUTHORIZATION)
      .and_then(|h| h.to_str().ok());

    let token = auth_header.and_then(|h| h.strip_prefix("Bearer "));

    if token != Some(&state.token) {
      return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(req).await)
  }

  async fn handle_mcp_post(
    State(state): State<McpHttpState>,
    Json(request): Json<McpRequest>,
  ) -> impl IntoResponse {
    let response = state.server.handle_request(request).await;
    Json(response)
  }

  pub async fn stop(&self) -> Result<(), String> {
    if !self.is_running() {
      return Err("MCP server is not running".to_string());
    }

    let mut inner = self.inner.lock().await;
    inner.app_handle = None;
    inner.token = None;

    // Send shutdown signal
    if let Some(tx) = inner.shutdown_tx.take() {
      let _ = tx.send(());
    }

    self.port.store(0, Ordering::SeqCst);
    self.is_running.store(false, Ordering::SeqCst);

    log::info!("[mcp] Server stopped");
    Ok(())
  }

  pub fn get_tools(&self) -> Vec<McpTool> {
    vec![
      McpTool {
        name: "list_profiles".to_string(),
        description: "List all Wayfern and Camoufox browser profiles".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "get_profile".to_string(),
        description: "Get details of a specific browser profile".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to retrieve"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "run_profile".to_string(),
        description: "Launch a browser profile with an optional URL".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to launch"
            },
            "url": {
              "type": "string",
              "description": "Optional URL to open in the browser"
            },
            "headless": {
              "type": "boolean",
              "description": "Run the browser in headless mode"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "kill_profile".to_string(),
        description: "Stop a running browser profile".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to stop"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "list_proxies".to_string(),
        description: "List all configured proxies".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "get_profile_status".to_string(),
        description: "Check if a browser profile is currently running".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to check"
            }
          },
          "required": ["profile_id"]
        }),
      },
      // Group management tools
      McpTool {
        name: "list_groups".to_string(),
        description: "List all profile groups".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "get_group".to_string(),
        description: "Get details of a specific group".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "group_id": {
              "type": "string",
              "description": "The UUID of the group to retrieve"
            }
          },
          "required": ["group_id"]
        }),
      },
      McpTool {
        name: "create_group".to_string(),
        description: "Create a new profile group".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "name": {
              "type": "string",
              "description": "The name for the new group"
            }
          },
          "required": ["name"]
        }),
      },
      McpTool {
        name: "update_group".to_string(),
        description: "Update an existing group's name".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "group_id": {
              "type": "string",
              "description": "The UUID of the group to update"
            },
            "name": {
              "type": "string",
              "description": "The new name for the group"
            }
          },
          "required": ["group_id", "name"]
        }),
      },
      McpTool {
        name: "delete_group".to_string(),
        description: "Delete a profile group".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "group_id": {
              "type": "string",
              "description": "The UUID of the group to delete"
            }
          },
          "required": ["group_id"]
        }),
      },
      McpTool {
        name: "assign_profiles_to_group".to_string(),
        description: "Assign one or more profiles to a group".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_ids": {
              "type": "array",
              "items": { "type": "string" },
              "description": "Array of profile UUIDs to assign"
            },
            "group_id": {
              "type": "string",
              "description": "The UUID of the group to assign to (null to remove from group)"
            }
          },
          "required": ["profile_ids"]
        }),
      },
      // Full proxy management tools
      McpTool {
        name: "get_proxy".to_string(),
        description: "Get details of a specific proxy".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "proxy_id": {
              "type": "string",
              "description": "The UUID of the proxy to retrieve"
            }
          },
          "required": ["proxy_id"]
        }),
      },
      McpTool {
        name: "create_proxy".to_string(),
        description: "Create a new proxy configuration".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "name": {
              "type": "string",
              "description": "The name for the new proxy"
            },
            "proxy_type": {
              "type": "string",
              "enum": ["http", "https", "socks4", "socks5"],
              "description": "The type of proxy"
            },
            "host": {
              "type": "string",
              "description": "The proxy host address"
            },
            "port": {
              "type": "integer",
              "description": "The proxy port number"
            },
            "username": {
              "type": "string",
              "description": "Optional username for authentication"
            },
            "password": {
              "type": "string",
              "description": "Optional password for authentication"
            }
          },
          "required": ["name", "proxy_type", "host", "port"]
        }),
      },
      McpTool {
        name: "update_proxy".to_string(),
        description: "Update an existing proxy configuration".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "proxy_id": {
              "type": "string",
              "description": "The UUID of the proxy to update"
            },
            "name": {
              "type": "string",
              "description": "New name for the proxy"
            },
            "proxy_type": {
              "type": "string",
              "enum": ["http", "https", "socks4", "socks5"],
              "description": "The type of proxy"
            },
            "host": {
              "type": "string",
              "description": "The proxy host address"
            },
            "port": {
              "type": "integer",
              "description": "The proxy port number"
            },
            "username": {
              "type": "string",
              "description": "Optional username for authentication"
            },
            "password": {
              "type": "string",
              "description": "Optional password for authentication"
            }
          },
          "required": ["proxy_id"]
        }),
      },
      McpTool {
        name: "delete_proxy".to_string(),
        description: "Delete a proxy configuration".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "proxy_id": {
              "type": "string",
              "description": "The UUID of the proxy to delete"
            }
          },
          "required": ["proxy_id"]
        }),
      },
      McpTool {
        name: "export_proxies".to_string(),
        description: "Export all proxy configurations".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "format": {
              "type": "string",
              "enum": ["json", "txt"],
              "description": "Export format (json for structured data, txt for URL format)"
            }
          },
          "required": ["format"]
        }),
      },
      McpTool {
        name: "import_proxies".to_string(),
        description: "Import proxy configurations from JSON or TXT content".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "content": {
              "type": "string",
              "description": "The proxy configuration content to import"
            },
            "format": {
              "type": "string",
              "enum": ["json", "txt"],
              "description": "Import format (json or txt)"
            },
            "name_prefix": {
              "type": "string",
              "description": "Optional prefix for imported proxy names (default: 'Imported')"
            }
          },
          "required": ["content", "format"]
        }),
      },
      // VPN management tools
      McpTool {
        name: "import_vpn".to_string(),
        description: "Import a WireGuard (.conf) or OpenVPN (.ovpn) configuration".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "content": {
              "type": "string",
              "description": "Raw VPN config file content"
            },
            "filename": {
              "type": "string",
              "description": "Original filename (.conf or .ovpn) for type detection"
            },
            "name": {
              "type": "string",
              "description": "Optional display name for the VPN config"
            }
          },
          "required": ["content", "filename"]
        }),
      },
      McpTool {
        name: "list_vpn_configs".to_string(),
        description: "List all stored VPN configurations".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "delete_vpn".to_string(),
        description: "Delete a VPN configuration".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "vpn_id": {
              "type": "string",
              "description": "The UUID of the VPN config to delete"
            }
          },
          "required": ["vpn_id"]
        }),
      },
      McpTool {
        name: "connect_vpn".to_string(),
        description: "Connect to a VPN configuration".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "vpn_id": {
              "type": "string",
              "description": "The UUID of the VPN config to connect"
            }
          },
          "required": ["vpn_id"]
        }),
      },
      McpTool {
        name: "disconnect_vpn".to_string(),
        description: "Disconnect from a VPN".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "vpn_id": {
              "type": "string",
              "description": "The UUID of the VPN to disconnect"
            }
          },
          "required": ["vpn_id"]
        }),
      },
      McpTool {
        name: "get_vpn_status".to_string(),
        description: "Get the connection status of a VPN".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "vpn_id": {
              "type": "string",
              "description": "The UUID of the VPN to check"
            }
          },
          "required": ["vpn_id"]
        }),
      },
      // Fingerprint management tools
      McpTool {
        name: "get_profile_fingerprint".to_string(),
        description: "Get the fingerprint configuration for a Wayfern or Camoufox profile"
          .to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "update_profile_fingerprint".to_string(),
        description:
          "Update the fingerprint configuration for a Wayfern or Camoufox profile. Requires an active Pro subscription."
            .to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": {
              "type": "string",
              "description": "The UUID of the profile to update"
            },
            "fingerprint": {
              "type": "string",
              "description": "JSON string of the fingerprint configuration, or null to clear"
            },
            "os": {
              "type": "string",
              "enum": ["windows", "macos", "linux"],
              "description": "Operating system for fingerprint generation"
            },
            "randomize_fingerprint_on_launch": {
              "type": "boolean",
              "description": "Whether to generate a new fingerprint on every launch"
            }
          },
          "required": ["profile_id"]
        }),
      },
      McpTool {
        name: "list_extensions".to_string(),
        description: "List all managed browser extensions. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "list_extension_groups".to_string(),
        description: "List all extension groups. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {},
          "required": []
        }),
      },
      McpTool {
        name: "create_extension_group".to_string(),
        description: "Create a new extension group. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "name": { "type": "string", "description": "Name for the extension group" }
          },
          "required": ["name"]
        }),
      },
      McpTool {
        name: "delete_extension".to_string(),
        description: "Delete a managed extension. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "extension_id": { "type": "string", "description": "The extension ID to delete" }
          },
          "required": ["extension_id"]
        }),
      },
      McpTool {
        name: "delete_extension_group".to_string(),
        description: "Delete an extension group. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "group_id": { "type": "string", "description": "The extension group ID to delete" }
          },
          "required": ["group_id"]
        }),
      },
      McpTool {
        name: "assign_extension_group_to_profile".to_string(),
        description: "Assign an extension group to a profile, or remove the assignment. Requires Pro subscription.".to_string(),
        input_schema: serde_json::json!({
          "type": "object",
          "properties": {
            "profile_id": { "type": "string", "description": "The profile ID" },
            "extension_group_id": { "type": "string", "description": "The extension group ID, or empty string to remove" }
          },
          "required": ["profile_id"]
        }),
      },
    ]
  }

  pub async fn handle_request(&self, request: McpRequest) -> McpResponse {
    if !self.is_running() {
      return McpResponse {
        jsonrpc: "2.0".to_string(),
        id: request.id,
        result: None,
        error: Some(McpError {
          code: -32001,
          message: "MCP server is not running".to_string(),
        }),
      };
    }

    let result = match request.method.as_str() {
      "tools/list" => self.handle_tools_list().await,
      "tools/call" => self.handle_tool_call(request.params).await,
      _ => Err(McpError {
        code: -32601,
        message: format!("Method not found: {}", request.method),
      }),
    };

    match result {
      Ok(value) => McpResponse {
        jsonrpc: "2.0".to_string(),
        id: request.id,
        result: Some(value),
        error: None,
      },
      Err(error) => McpResponse {
        jsonrpc: "2.0".to_string(),
        id: request.id,
        result: None,
        error: Some(error),
      },
    }
  }

  async fn handle_tools_list(&self) -> Result<serde_json::Value, McpError> {
    Ok(serde_json::json!({
      "tools": self.get_tools()
    }))
  }

  async fn handle_tool_call(
    &self,
    params: Option<serde_json::Value>,
  ) -> Result<serde_json::Value, McpError> {
    let params = params.ok_or_else(|| McpError {
      code: -32602,
      message: "Missing parameters".to_string(),
    })?;

    let tool_name = params
      .get("name")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing tool name".to_string(),
      })?;

    let arguments = params
      .get("arguments")
      .cloned()
      .unwrap_or(serde_json::json!({}));

    match tool_name {
      "list_profiles" => self.handle_list_profiles().await,
      "get_profile" => self.handle_get_profile(&arguments).await,
      "run_profile" => self.handle_run_profile(&arguments).await,
      "kill_profile" => self.handle_kill_profile(&arguments).await,
      "list_proxies" => self.handle_list_proxies().await,
      "get_profile_status" => self.handle_get_profile_status(&arguments).await,
      // Group management
      "list_groups" => self.handle_list_groups().await,
      "get_group" => self.handle_get_group(&arguments).await,
      "create_group" => self.handle_create_group(&arguments).await,
      "update_group" => self.handle_update_group(&arguments).await,
      "delete_group" => self.handle_delete_group(&arguments).await,
      "assign_profiles_to_group" => self.handle_assign_profiles_to_group(&arguments).await,
      // Full proxy management
      "get_proxy" => self.handle_get_proxy(&arguments).await,
      "create_proxy" => self.handle_create_proxy(&arguments).await,
      "update_proxy" => self.handle_update_proxy(&arguments).await,
      "delete_proxy" => self.handle_delete_proxy(&arguments).await,
      // Proxy import/export
      "export_proxies" => self.handle_export_proxies(&arguments).await,
      "import_proxies" => self.handle_import_proxies(&arguments).await,
      // VPN management
      "import_vpn" => self.handle_import_vpn(&arguments).await,
      "list_vpn_configs" => self.handle_list_vpn_configs().await,
      "delete_vpn" => self.handle_delete_vpn(&arguments).await,
      "connect_vpn" => self.handle_connect_vpn(&arguments).await,
      "disconnect_vpn" => self.handle_disconnect_vpn(&arguments).await,
      "get_vpn_status" => self.handle_get_vpn_status(&arguments).await,
      // Fingerprint management
      "get_profile_fingerprint" => self.handle_get_profile_fingerprint(&arguments).await,
      "update_profile_fingerprint" => self.handle_update_profile_fingerprint(&arguments).await,
      // Extension management
      "list_extensions" => self.handle_list_extensions().await,
      "list_extension_groups" => self.handle_list_extension_groups().await,
      "create_extension_group" => self.handle_create_extension_group(&arguments).await,
      "delete_extension" => self.handle_delete_extension_mcp(&arguments).await,
      "delete_extension_group" => self.handle_delete_extension_group_mcp(&arguments).await,
      "assign_extension_group_to_profile" => {
        self
          .handle_assign_extension_group_to_profile(&arguments)
          .await
      }
      _ => Err(McpError {
        code: -32602,
        message: format!("Unknown tool: {tool_name}"),
      }),
    }
  }

  async fn handle_list_profiles(&self) -> Result<serde_json::Value, McpError> {
    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
      })?;

    // Filter to only Wayfern and Camoufox profiles
    let filtered: Vec<&BrowserProfile> = profiles
      .iter()
      .filter(|p| p.browser == "wayfern" || p.browser == "camoufox")
      .collect();

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&filtered).unwrap_or_default()
      }]
    }))
  }

  async fn handle_get_profile(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
      })?;

    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
      })?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Profile not found: {profile_id}"),
      })?;

    // Check if it's a Wayfern or Camoufox profile
    if profile.browser != "wayfern" && profile.browser != "camoufox" {
      return Err(McpError {
        code: -32000,
        message: "MCP only supports Wayfern and Camoufox profiles".to_string(),
      });
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&profile).unwrap_or_default()
      }]
    }))
  }

  async fn handle_run_profile(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
      })?;

    let url = arguments.get("url").and_then(|v| v.as_str());
    let _headless = arguments
      .get("headless")
      .and_then(|v| v.as_bool())
      .unwrap_or(false);

    // Get the profile
    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
      })?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Profile not found: {profile_id}"),
      })?;

    // Check if it's a Wayfern or Camoufox profile
    if profile.browser != "wayfern" && profile.browser != "camoufox" {
      return Err(McpError {
        code: -32000,
        message: "MCP only supports Wayfern and Camoufox profiles".to_string(),
      });
    }

    // Get app handle to launch
    let inner = self.inner.lock().await;
    let app_handle = inner.app_handle.as_ref().ok_or_else(|| McpError {
      code: -32000,
      message: "MCP server not properly initialized".to_string(),
    })?;

    // Launch the browser
    crate::browser_runner::BrowserRunner::instance()
      .launch_browser(
        app_handle.clone(),
        profile,
        url.map(|s| s.to_string()),
        None,
      )
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to launch browser: {e}"),
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Browser profile '{}' launched successfully", profile.name)
      }]
    }))
  }

  async fn handle_kill_profile(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
      })?;

    // Get the profile
    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
      })?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Profile not found: {profile_id}"),
      })?;

    // Check if it's a Wayfern or Camoufox profile
    if profile.browser != "wayfern" && profile.browser != "camoufox" {
      return Err(McpError {
        code: -32000,
        message: "MCP only supports Wayfern and Camoufox profiles".to_string(),
      });
    }

    // Get app handle to kill
    let inner = self.inner.lock().await;
    let app_handle = inner.app_handle.as_ref().ok_or_else(|| McpError {
      code: -32000,
      message: "MCP server not properly initialized".to_string(),
    })?;

    // Kill the browser
    crate::browser_runner::BrowserRunner::instance()
      .kill_browser_process(app_handle.clone(), profile)
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to kill browser: {e}"),
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Browser profile '{}' stopped successfully", profile.name)
      }]
    }))
  }

  async fn handle_list_proxies(&self) -> Result<serde_json::Value, McpError> {
    let proxies = PROXY_MANAGER.get_stored_proxies();

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&proxies).unwrap_or_default()
      }]
    }))
  }

  async fn handle_get_profile_status(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
      })?;

    // Get the profile
    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
      })?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Profile not found: {profile_id}"),
      })?;

    // Check if it's a Wayfern or Camoufox profile
    if profile.browser != "wayfern" && profile.browser != "camoufox" {
      return Err(McpError {
        code: -32000,
        message: "MCP only supports Wayfern and Camoufox profiles".to_string(),
      });
    }

    let is_running = profile.process_id.is_some();

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::json!({
          "profile_id": profile_id,
          "is_running": is_running
        }).to_string()
      }]
    }))
  }

  // Group management handlers
  async fn handle_list_groups(&self) -> Result<serde_json::Value, McpError> {
    let groups = GROUP_MANAGER
      .lock()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to lock group manager: {e}"),
      })?
      .get_all_groups()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list groups: {e}"),
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&groups).unwrap_or_default()
      }]
    }))
  }

  async fn handle_get_group(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let group_id = arguments
      .get("group_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing group_id".to_string(),
      })?;

    let groups = GROUP_MANAGER
      .lock()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to lock group manager: {e}"),
      })?
      .get_all_groups()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list groups: {e}"),
      })?;

    let group = groups
      .iter()
      .find(|g| g.id == group_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Group not found: {group_id}"),
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&group).unwrap_or_default()
      }]
    }))
  }

  async fn handle_create_group(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let name = arguments
      .get("name")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing name".to_string(),
      })?;

    let inner = self.inner.lock().await;
    let app_handle = inner.app_handle.as_ref().ok_or_else(|| McpError {
      code: -32000,
      message: "MCP server not properly initialized".to_string(),
    })?;

    let group = GROUP_MANAGER
      .lock()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to lock group manager: {e}"),
      })?
      .create_group(app_handle, name.to_string())
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to create group: {e}"),
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Group '{}' created successfully with ID: {}", group.name, group.id)
      }]
    }))
  }

  async fn handle_update_group(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let group_id = arguments
      .get("group_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing group_id".to_string(),
      })?;

    let name = arguments
      .get("name")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing name".to_string(),
      })?;

    let inner = self.inner.lock().await;
    let app_handle = inner.app_handle.as_ref().ok_or_else(|| McpError {
      code: -32000,
      message: "MCP server not properly initialized".to_string(),
    })?;

    let group = GROUP_MANAGER
      .lock()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to lock group manager: {e}"),
      })?
      .update_group(app_handle, group_id.to_string(), name.to_string())
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to update group: {e}"),
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Group '{}' updated successfully", group.name)
      }]
    }))
  }

  async fn handle_delete_group(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let group_id = arguments
      .get("group_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing group_id".to_string(),
      })?;

    let inner = self.inner.lock().await;
    let app_handle = inner.app_handle.as_ref().ok_or_else(|| McpError {
      code: -32000,
      message: "MCP server not properly initialized".to_string(),
    })?;

    GROUP_MANAGER
      .lock()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to lock group manager: {e}"),
      })?
      .delete_group(app_handle, group_id.to_string())
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to delete group: {e}"),
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Group '{}' deleted successfully", group_id)
      }]
    }))
  }

  async fn handle_assign_profiles_to_group(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_ids: Vec<String> = arguments
      .get("profile_ids")
      .and_then(|v| v.as_array())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_ids".to_string(),
      })?
      .iter()
      .filter_map(|v| v.as_str().map(|s| s.to_string()))
      .collect();

    let group_id = arguments
      .get("group_id")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());

    let inner = self.inner.lock().await;
    let app_handle = inner.app_handle.as_ref().ok_or_else(|| McpError {
      code: -32000,
      message: "MCP server not properly initialized".to_string(),
    })?;

    ProfileManager::instance()
      .assign_profiles_to_group(app_handle, profile_ids.clone(), group_id.clone())
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to assign profiles to group: {e}"),
      })?;

    let group_name = group_id.as_deref().unwrap_or("default");
    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("{} profile(s) assigned to group '{}'", profile_ids.len(), group_name)
      }]
    }))
  }

  // Full proxy management handlers
  async fn handle_get_proxy(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let proxy_id = arguments
      .get("proxy_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing proxy_id".to_string(),
      })?;

    let proxies = PROXY_MANAGER.get_stored_proxies();
    let proxy = proxies
      .iter()
      .find(|p| p.id == proxy_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Proxy not found: {proxy_id}"),
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&proxy).unwrap_or_default()
      }]
    }))
  }

  async fn handle_create_proxy(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let name = arguments
      .get("name")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing name".to_string(),
      })?;

    let proxy_type = arguments
      .get("proxy_type")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing proxy_type".to_string(),
      })?;

    let host = arguments
      .get("host")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing host".to_string(),
      })?;

    let port = arguments
      .get("port")
      .and_then(|v| v.as_u64())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing port".to_string(),
      })? as u16;

    let username = arguments
      .get("username")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());
    let password = arguments
      .get("password")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());

    let proxy_settings = ProxySettings {
      proxy_type: proxy_type.to_string(),
      host: host.to_string(),
      port,
      username,
      password,
    };

    let inner = self.inner.lock().await;
    let app_handle = inner.app_handle.as_ref().ok_or_else(|| McpError {
      code: -32000,
      message: "MCP server not properly initialized".to_string(),
    })?;

    let proxy = PROXY_MANAGER
      .create_stored_proxy(app_handle, name.to_string(), proxy_settings)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to create proxy: {e}"),
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Proxy '{}' created successfully with ID: {}", proxy.name, proxy.id)
      }]
    }))
  }

  async fn handle_update_proxy(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let proxy_id = arguments
      .get("proxy_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing proxy_id".to_string(),
      })?;

    let name = arguments
      .get("name")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());

    // Build proxy_settings if any settings fields are provided
    let has_settings = arguments.get("proxy_type").is_some()
      || arguments.get("host").is_some()
      || arguments.get("port").is_some();

    let proxy_settings = if has_settings {
      // Get existing proxy to use as defaults
      let proxies = PROXY_MANAGER.get_stored_proxies();
      let existing = proxies
        .iter()
        .find(|p| p.id == proxy_id)
        .ok_or_else(|| McpError {
          code: -32000,
          message: format!("Proxy not found: {proxy_id}"),
        })?;

      let proxy_type = arguments
        .get("proxy_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| existing.proxy_settings.proxy_type.clone());

      let host = arguments
        .get("host")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| existing.proxy_settings.host.clone());

      let port = arguments
        .get("port")
        .and_then(|v| v.as_u64())
        .map(|p| p as u16)
        .unwrap_or(existing.proxy_settings.port);

      let username = arguments
        .get("username")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| existing.proxy_settings.username.clone());

      let password = arguments
        .get("password")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| existing.proxy_settings.password.clone());

      Some(ProxySettings {
        proxy_type,
        host,
        port,
        username,
        password,
      })
    } else {
      None
    };

    let inner = self.inner.lock().await;
    let app_handle = inner.app_handle.as_ref().ok_or_else(|| McpError {
      code: -32000,
      message: "MCP server not properly initialized".to_string(),
    })?;

    let proxy = PROXY_MANAGER
      .update_stored_proxy(app_handle, proxy_id, name, proxy_settings)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to update proxy: {e}"),
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Proxy '{}' updated successfully", proxy.name)
      }]
    }))
  }

  async fn handle_delete_proxy(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let proxy_id = arguments
      .get("proxy_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing proxy_id".to_string(),
      })?;

    let inner = self.inner.lock().await;
    let app_handle = inner.app_handle.as_ref().ok_or_else(|| McpError {
      code: -32000,
      message: "MCP server not properly initialized".to_string(),
    })?;

    PROXY_MANAGER
      .delete_stored_proxy(app_handle, proxy_id)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to delete proxy: {e}"),
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Proxy '{}' deleted successfully", proxy_id)
      }]
    }))
  }

  async fn handle_export_proxies(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let format = arguments
      .get("format")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing format".to_string(),
      })?;

    let content = match format {
      "json" => PROXY_MANAGER.export_proxies_json().map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to export proxies: {e}"),
      })?,
      "txt" => PROXY_MANAGER.export_proxies_txt(),
      _ => {
        return Err(McpError {
          code: -32602,
          message: format!("Invalid format '{}', must be 'json' or 'txt'", format),
        })
      }
    };

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": content
      }]
    }))
  }

  async fn handle_import_proxies(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let content = arguments
      .get("content")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing content".to_string(),
      })?;

    let format = arguments
      .get("format")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing format".to_string(),
      })?;

    let name_prefix = arguments
      .get("name_prefix")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());

    let inner = self.inner.lock().await;
    let app_handle = inner.app_handle.as_ref().ok_or_else(|| McpError {
      code: -32000,
      message: "MCP server not properly initialized".to_string(),
    })?;

    let result = match format {
      "json" => PROXY_MANAGER
        .import_proxies_json(app_handle, content)
        .map_err(|e| McpError {
          code: -32000,
          message: format!("Failed to import proxies: {e}"),
        })?,
      "txt" => {
        use crate::proxy_manager::{ProxyManager, ProxyParseResult};

        let parse_results = ProxyManager::parse_txt_proxies(content);
        let parsed: Vec<_> = parse_results
          .into_iter()
          .filter_map(|r| {
            if let ProxyParseResult::Parsed(p) = r {
              Some(p)
            } else {
              None
            }
          })
          .collect();

        if parsed.is_empty() {
          return Err(McpError {
            code: -32000,
            message: "No valid proxies found in content".to_string(),
          });
        }

        PROXY_MANAGER
          .import_proxies_from_parsed(app_handle, parsed, name_prefix)
          .map_err(|e| McpError {
            code: -32000,
            message: format!("Failed to import proxies: {e}"),
          })?
      }
      _ => {
        return Err(McpError {
          code: -32602,
          message: format!("Invalid format '{}', must be 'json' or 'txt'", format),
        })
      }
    };

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!(
          "Import complete: {} imported, {} skipped, {} errors",
          result.imported_count,
          result.skipped_count,
          result.errors.len()
        )
      }]
    }))
  }

  // VPN management handlers
  async fn handle_import_vpn(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let content = arguments
      .get("content")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing content".to_string(),
      })?;

    let filename = arguments
      .get("filename")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing filename".to_string(),
      })?;

    let name = arguments
      .get("name")
      .and_then(|v| v.as_str())
      .map(|s| s.to_string());

    let storage = crate::vpn::VPN_STORAGE.lock().map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to lock VPN storage: {e}"),
    })?;

    let config = storage
      .import_config(content, filename, name)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to import VPN config: {e}"),
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!(
          "VPN '{}' ({}) imported successfully with ID: {}",
          config.name,
          config.vpn_type,
          config.id
        )
      }]
    }))
  }

  async fn handle_list_vpn_configs(&self) -> Result<serde_json::Value, McpError> {
    let storage = crate::vpn::VPN_STORAGE.lock().map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to lock VPN storage: {e}"),
    })?;

    let configs = storage.list_configs().map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to list VPN configs: {e}"),
    })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&configs).unwrap_or_default()
      }]
    }))
  }

  async fn handle_delete_vpn(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let vpn_id = arguments
      .get("vpn_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing vpn_id".to_string(),
      })?;

    // First disconnect if connected (stop VPN worker)
    let _ = crate::vpn_worker_runner::stop_vpn_worker_by_vpn_id(vpn_id).await;

    let storage = crate::vpn::VPN_STORAGE.lock().map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to lock VPN storage: {e}"),
    })?;

    storage.delete_config(vpn_id).map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to delete VPN config: {e}"),
    })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("VPN '{}' deleted successfully", vpn_id)
      }]
    }))
  }

  async fn handle_connect_vpn(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let vpn_id = arguments
      .get("vpn_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing vpn_id".to_string(),
      })?;

    // Start VPN worker process
    crate::vpn_worker_runner::start_vpn_worker(vpn_id)
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to connect VPN: {e}"),
      })?;

    // Update last_used timestamp
    {
      let storage = crate::vpn::VPN_STORAGE.lock().map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to lock VPN storage: {e}"),
      })?;
      let _ = storage.update_last_used(vpn_id);
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("VPN '{}' connected successfully", vpn_id)
      }]
    }))
  }

  async fn handle_disconnect_vpn(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let vpn_id = arguments
      .get("vpn_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing vpn_id".to_string(),
      })?;

    crate::vpn_worker_runner::stop_vpn_worker_by_vpn_id(vpn_id)
      .await
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to disconnect VPN: {e}"),
      })?;

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("VPN '{}' disconnected successfully", vpn_id)
      }]
    }))
  }

  async fn handle_get_vpn_status(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let vpn_id = arguments
      .get("vpn_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing vpn_id".to_string(),
      })?;

    let connected =
      if let Some(worker) = crate::vpn_worker_storage::find_vpn_worker_by_vpn_id(vpn_id) {
        worker
          .pid
          .map(crate::proxy_storage::is_process_running)
          .unwrap_or(false)
      } else {
        false
      };

    let status = crate::vpn::VpnStatus {
      connected,
      vpn_id: vpn_id.to_string(),
      connected_at: None,
      bytes_sent: None,
      bytes_received: None,
      last_handshake: None,
    };

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&status).unwrap_or_default()
      }]
    }))
  }

  // Fingerprint management handlers
  async fn handle_get_profile_fingerprint(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
      })?;

    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
      })?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Profile not found: {profile_id}"),
      })?;

    let fingerprint_info = match profile.browser.as_str() {
      "camoufox" => {
        let config = profile
          .camoufox_config
          .as_ref()
          .cloned()
          .unwrap_or_default();
        serde_json::json!({
          "browser": "camoufox",
          "fingerprint": config.fingerprint,
          "os": config.os,
          "randomize_fingerprint_on_launch": config.randomize_fingerprint_on_launch,
          "screen_max_width": config.screen_max_width,
          "screen_max_height": config.screen_max_height,
          "screen_min_width": config.screen_min_width,
          "screen_min_height": config.screen_min_height,
        })
      }
      "wayfern" => {
        let config = profile.wayfern_config.as_ref().cloned().unwrap_or_default();
        serde_json::json!({
          "browser": "wayfern",
          "fingerprint": config.fingerprint,
          "os": config.os,
          "randomize_fingerprint_on_launch": config.randomize_fingerprint_on_launch,
          "screen_max_width": config.screen_max_width,
          "screen_max_height": config.screen_max_height,
          "screen_min_width": config.screen_min_width,
          "screen_min_height": config.screen_min_height,
        })
      }
      _ => {
        return Err(McpError {
          code: -32000,
          message: "MCP only supports Wayfern and Camoufox profiles".to_string(),
        })
      }
    };

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": serde_json::to_string_pretty(&fingerprint_info).unwrap_or_default()
      }]
    }))
  }

  async fn handle_update_profile_fingerprint(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Fingerprint editing requires an active Pro subscription".to_string(),
      });
    }

    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing profile_id".to_string(),
      })?;

    let fingerprint = arguments.get("fingerprint").and_then(|v| v.as_str());
    let os = arguments.get("os").and_then(|v| v.as_str());
    let randomize = arguments
      .get("randomize_fingerprint_on_launch")
      .and_then(|v| v.as_bool());

    if let Some(os_val) = os {
      if !CLOUD_AUTH.is_fingerprint_os_allowed(Some(os_val)).await {
        return Err(McpError {
          code: -32000,
          message: format!(
            "OS spoofing to '{}' requires an active Pro subscription",
            os_val
          ),
        });
      }
    }

    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
      })?;

    let profile = profiles
      .iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or_else(|| McpError {
        code: -32000,
        message: format!("Profile not found: {profile_id}"),
      })?;

    let inner = self.inner.lock().await;
    let app_handle = inner.app_handle.as_ref().ok_or_else(|| McpError {
      code: -32000,
      message: "MCP server not properly initialized".to_string(),
    })?;

    match profile.browser.as_str() {
      "camoufox" => {
        let mut config = profile
          .camoufox_config
          .as_ref()
          .cloned()
          .unwrap_or_default();
        if let Some(fp) = fingerprint {
          config.fingerprint = Some(fp.to_string());
        }
        if let Some(os_val) = os {
          config.os = Some(os_val.to_string());
        }
        if let Some(r) = randomize {
          config.randomize_fingerprint_on_launch = Some(r);
        }
        ProfileManager::instance()
          .update_camoufox_config(app_handle.clone(), profile_id, config)
          .await
          .map_err(|e| McpError {
            code: -32000,
            message: format!("Failed to update camoufox config: {e}"),
          })?;
      }
      "wayfern" => {
        let mut config = profile.wayfern_config.as_ref().cloned().unwrap_or_default();
        if let Some(fp) = fingerprint {
          config.fingerprint = Some(fp.to_string());
        }
        if let Some(os_val) = os {
          config.os = Some(os_val.to_string());
        }
        if let Some(r) = randomize {
          config.randomize_fingerprint_on_launch = Some(r);
        }
        ProfileManager::instance()
          .update_wayfern_config(app_handle.clone(), profile_id, config)
          .await
          .map_err(|e| McpError {
            code: -32000,
            message: format!("Failed to update wayfern config: {e}"),
          })?;
      }
      _ => {
        return Err(McpError {
          code: -32000,
          message: "MCP only supports Wayfern and Camoufox profiles".to_string(),
        })
      }
    }

    Ok(serde_json::json!({
      "content": [{
        "type": "text",
        "text": format!("Fingerprint configuration updated for profile '{}'", profile.name)
      }]
    }))
  }

  async fn handle_list_extensions(&self) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
      });
    }
    let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    let extensions = mgr.list_extensions().map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to list extensions: {e}"),
    })?;
    Ok(serde_json::to_value(extensions).unwrap())
  }

  async fn handle_list_extension_groups(&self) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
      });
    }
    let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    let groups = mgr.list_groups().map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to list extension groups: {e}"),
    })?;
    Ok(serde_json::to_value(groups).unwrap())
  }

  async fn handle_create_extension_group(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
      });
    }
    let name = arguments
      .get("name")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing required parameter: name".to_string(),
      })?;
    let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    let group = mgr.create_group(name.to_string()).map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to create extension group: {e}"),
    })?;
    Ok(serde_json::to_value(group).unwrap())
  }

  async fn handle_delete_extension_mcp(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
      });
    }
    let extension_id = arguments
      .get("extension_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing required parameter: extension_id".to_string(),
      })?;
    let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    mgr
      .delete_extension_internal(extension_id)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to delete extension: {e}"),
      })?;
    Ok(serde_json::json!({"success": true}))
  }

  async fn handle_delete_extension_group_mcp(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
      });
    }
    let group_id = arguments
      .get("group_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing required parameter: group_id".to_string(),
      })?;
    let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
    // For MCP, we don't have an app_handle, but we need one for sync deletion.
    // Use the delete_group_internal which skips sync remote deletion.
    mgr.delete_group_internal(group_id).map_err(|e| McpError {
      code: -32000,
      message: format!("Failed to delete extension group: {e}"),
    })?;
    if let Err(e) = crate::events::emit_empty("extensions-changed") {
      log::error!("Failed to emit extensions-changed event: {e}");
    }
    Ok(serde_json::json!({"success": true}))
  }

  async fn handle_assign_extension_group_to_profile(
    &self,
    arguments: &serde_json::Value,
  ) -> Result<serde_json::Value, McpError> {
    if !CLOUD_AUTH.has_active_paid_subscription().await {
      return Err(McpError {
        code: -32000,
        message: "Extension management requires an active Pro subscription".to_string(),
      });
    }
    let profile_id = arguments
      .get("profile_id")
      .and_then(|v| v.as_str())
      .ok_or_else(|| McpError {
        code: -32602,
        message: "Missing required parameter: profile_id".to_string(),
      })?;
    let extension_group_id = arguments
      .get("extension_group_id")
      .and_then(|v| v.as_str())
      .map(|s| {
        if s.is_empty() {
          None
        } else {
          Some(s.to_string())
        }
      })
      .unwrap_or(None);

    // Validate compatibility if assigning
    if let Some(ref gid) = extension_group_id {
      let profile_manager = ProfileManager::instance();
      let profiles = profile_manager.list_profiles().map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to list profiles: {e}"),
      })?;
      let profile = profiles
        .iter()
        .find(|p| p.id.to_string() == profile_id)
        .ok_or_else(|| McpError {
          code: -32000,
          message: format!("Profile '{profile_id}' not found"),
        })?;
      let mgr = crate::extension_manager::EXTENSION_MANAGER.lock().unwrap();
      mgr
        .validate_group_compatibility(gid, &profile.browser)
        .map_err(|e| McpError {
          code: -32000,
          message: format!("{e}"),
        })?;
    }

    let profile_manager = ProfileManager::instance();
    let profile = profile_manager
      .update_profile_extension_group(profile_id, extension_group_id)
      .map_err(|e| McpError {
        code: -32000,
        message: format!("Failed to assign extension group: {e}"),
      })?;
    Ok(serde_json::to_value(profile).unwrap())
  }
}

lazy_static::lazy_static! {
  static ref MCP_SERVER: McpServer = McpServer::new();
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_mcp_tools_count() {
    let server = McpServer::new();
    let tools = server.get_tools();

    // Should have at least 32 tools (26 + 6 extension tools)
    assert!(tools.len() >= 32);

    // Check tool names
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    // Profile tools
    assert!(tool_names.contains(&"list_profiles"));
    assert!(tool_names.contains(&"get_profile"));
    assert!(tool_names.contains(&"run_profile"));
    assert!(tool_names.contains(&"kill_profile"));
    assert!(tool_names.contains(&"get_profile_status"));
    // Group tools
    assert!(tool_names.contains(&"list_groups"));
    assert!(tool_names.contains(&"get_group"));
    assert!(tool_names.contains(&"create_group"));
    assert!(tool_names.contains(&"update_group"));
    assert!(tool_names.contains(&"delete_group"));
    assert!(tool_names.contains(&"assign_profiles_to_group"));
    // Proxy tools
    assert!(tool_names.contains(&"list_proxies"));
    assert!(tool_names.contains(&"get_proxy"));
    assert!(tool_names.contains(&"create_proxy"));
    assert!(tool_names.contains(&"update_proxy"));
    assert!(tool_names.contains(&"delete_proxy"));
    // Proxy import/export tools
    assert!(tool_names.contains(&"export_proxies"));
    assert!(tool_names.contains(&"import_proxies"));
    // VPN tools
    assert!(tool_names.contains(&"import_vpn"));
    assert!(tool_names.contains(&"list_vpn_configs"));
    assert!(tool_names.contains(&"delete_vpn"));
    assert!(tool_names.contains(&"connect_vpn"));
    assert!(tool_names.contains(&"disconnect_vpn"));
    assert!(tool_names.contains(&"get_vpn_status"));
    // Fingerprint tools
    assert!(tool_names.contains(&"get_profile_fingerprint"));
    assert!(tool_names.contains(&"update_profile_fingerprint"));
    // Extension tools
    assert!(tool_names.contains(&"list_extensions"));
    assert!(tool_names.contains(&"list_extension_groups"));
    assert!(tool_names.contains(&"create_extension_group"));
    assert!(tool_names.contains(&"delete_extension"));
    assert!(tool_names.contains(&"delete_extension_group"));
    assert!(tool_names.contains(&"assign_extension_group_to_profile"));
  }

  #[test]
  fn test_mcp_server_initial_state() {
    let server = McpServer::new();
    assert!(!server.is_running());
  }
}
