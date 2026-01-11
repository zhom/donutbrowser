#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::Mutex as AsyncMutex;

use crate::browser::ProxySettings;
use crate::group_manager::GROUP_MANAGER;
use crate::profile::{BrowserProfile, ProfileManager};
use crate::proxy_manager::PROXY_MANAGER;
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

struct McpServerInner {
  app_handle: Option<AppHandle>,
}

pub struct McpServer {
  inner: Arc<AsyncMutex<McpServerInner>>,
  is_running: AtomicBool,
}

impl McpServer {
  fn new() -> Self {
    Self {
      inner: Arc::new(AsyncMutex::new(McpServerInner { app_handle: None })),
      is_running: AtomicBool::new(false),
    }
  }

  pub fn instance() -> &'static McpServer {
    &MCP_SERVER
  }

  pub fn is_running(&self) -> bool {
    self.is_running.load(Ordering::SeqCst)
  }

  pub async fn start(&self, app_handle: AppHandle) -> Result<(), String> {
    // Check terms acceptance first
    if !WayfernTermsManager::instance().is_terms_accepted() {
      return Err(
        "Wayfern Terms and Conditions must be accepted before starting MCP server".to_string(),
      );
    }

    if self.is_running() {
      return Err("MCP server is already running".to_string());
    }

    let mut inner = self.inner.lock().await;
    inner.app_handle = Some(app_handle);
    self.is_running.store(true, Ordering::SeqCst);

    log::info!("MCP server started");
    Ok(())
  }

  pub async fn stop(&self) -> Result<(), String> {
    if !self.is_running() {
      return Err("MCP server is not running".to_string());
    }

    let mut inner = self.inner.lock().await;
    inner.app_handle = None;
    self.is_running.store(false, Ordering::SeqCst);

    log::info!("MCP server stopped");
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

    // Should have all 16 tools
    assert!(tools.len() >= 16);

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
  }

  #[test]
  fn test_mcp_server_initial_state() {
    let server = McpServer::new();
    assert!(!server.is_running());
  }
}
