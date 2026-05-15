// MCP client integrations — installs/removes the donut-browser MCP server in
// 14 popular AI assistant clients. Ports the add-mcp registry to Rust.
//
// Claude Desktop is managed via Claude's local extensions bundle
// (manifest.json + node bridge), since the desktop app supports only stdio
// servers via its plain JSON config but exposes HTTP through the extension
// framework. See `add_mcp_to_claude_desktop_internal` in lib.rs. All other
// agents (including Claude Code) use the generic config-file installer here.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const SERVER_NAME: &str = "donut-browser";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum AgentCategory {
  DesktopApp,
  Cli,
  Editor,
  EditorExt,
}

#[derive(Debug, Clone, Copy)]
enum ConfigFormat {
  Json,
  Toml,
  Yaml,
}

#[derive(Debug, Clone)]
struct AgentSpec {
  id: &'static str,
  display_name: &'static str,
  category: AgentCategory,
  /// Top-level key (supports dot notation) where the server is written.
  config_key: &'static str,
  format: ConfigFormat,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpAgentInfo {
  pub id: String,
  pub display_name: String,
  pub category: AgentCategory,
  pub connected: bool,
  /// True when the underlying client appears to be installed on the system
  /// (its config directory exists), regardless of whether we have installed
  /// the donut-browser server into it.
  pub detected: bool,
}

fn home() -> Option<PathBuf> {
  dirs::home_dir()
}

#[cfg(target_os = "macos")]
fn vscode_user_dir() -> Option<PathBuf> {
  home().map(|h| {
    h.join("Library")
      .join("Application Support")
      .join("Code")
      .join("User")
  })
}

#[cfg(target_os = "windows")]
fn vscode_user_dir() -> Option<PathBuf> {
  std::env::var("APPDATA")
    .ok()
    .map(|a| PathBuf::from(a).join("Code").join("User"))
}

#[cfg(target_os = "linux")]
fn vscode_user_dir() -> Option<PathBuf> {
  let base = std::env::var("XDG_CONFIG_HOME")
    .ok()
    .map(PathBuf::from)
    .or_else(|| home().map(|h| h.join(".config")))?;
  Some(base.join("Code").join("User"))
}

#[cfg(target_os = "macos")]
fn zed_config_dir() -> Option<PathBuf> {
  home().map(|h| h.join("Library").join("Application Support").join("Zed"))
}

#[cfg(target_os = "windows")]
fn zed_config_dir() -> Option<PathBuf> {
  std::env::var("APPDATA")
    .ok()
    .map(|a| PathBuf::from(a).join("Zed"))
}

#[cfg(target_os = "linux")]
fn zed_config_dir() -> Option<PathBuf> {
  let base = std::env::var("XDG_CONFIG_HOME")
    .ok()
    .map(PathBuf::from)
    .or_else(|| home().map(|h| h.join(".config")))?;
  Some(base.join("zed"))
}

#[cfg(target_os = "windows")]
fn goose_config_path() -> Option<PathBuf> {
  std::env::var("APPDATA").ok().map(|a| {
    PathBuf::from(a)
      .join("Block")
      .join("goose")
      .join("config")
      .join("config.yaml")
  })
}

#[cfg(not(target_os = "windows"))]
fn goose_config_path() -> Option<PathBuf> {
  let base = std::env::var("XDG_CONFIG_HOME")
    .ok()
    .map(PathBuf::from)
    .or_else(|| home().map(|h| h.join(".config")))?;
  Some(base.join("goose").join("config.yaml"))
}

/// Resolve the global config path for an agent. Returns `None` on unsupported
/// platforms (none currently — every supported agent has a defined path on
/// macOS/Linux/Windows).
fn config_path_for(agent_id: &str) -> Option<PathBuf> {
  let h = home()?;
  match agent_id {
    "antigravity" => Some(
      h.join(".gemini")
        .join("antigravity")
        .join("mcp_config.json"),
    ),
    "cline" => vscode_user_dir().map(|d| {
      d.join("globalStorage")
        .join("saoudrizwan.claude-dev")
        .join("settings")
        .join("cline_mcp_settings.json")
    }),
    "cline-cli" => {
      let base = std::env::var("CLINE_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| h.join(".cline"));
      Some(
        base
          .join("data")
          .join("settings")
          .join("cline_mcp_settings.json"),
      )
    }
    "claude-code" => Some(h.join(".claude.json")),
    "claude-desktop" => claude_desktop_config_path(),
    "codex" => {
      let base = std::env::var("CODEX_HOME")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| h.join(".codex"));
      Some(base.join("config.toml"))
    }
    "cursor" => Some(h.join(".cursor").join("mcp.json")),
    "gemini-cli" => Some(h.join(".gemini").join("settings.json")),
    "goose" => goose_config_path(),
    "github-copilot-cli" => Some(
      std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| h.join(".copilot"))
        .join("mcp-config.json"),
    ),
    "mcporter" => {
      // add-mcp's resolveMcporterConfigPath: prefer mcporter.json, fall back
      // to mcporter.jsonc if it already exists, else default to mcporter.json.
      let dir = h.join(".mcporter");
      let json_path = dir.join("mcporter.json");
      let jsonc_path = dir.join("mcporter.jsonc");
      if json_path.exists() {
        Some(json_path)
      } else if jsonc_path.exists() {
        Some(jsonc_path)
      } else {
        Some(json_path)
      }
    }
    "opencode" => Some(h.join(".config").join("opencode").join("opencode.json")),
    "vscode" => vscode_user_dir().map(|d| d.join("mcp.json")),
    "zed" => zed_config_dir().map(|d| d.join("settings.json")),
    _ => None,
  }
}

#[cfg(target_os = "macos")]
fn claude_desktop_config_path() -> Option<PathBuf> {
  home().map(|h| {
    h.join("Library")
      .join("Application Support")
      .join("Claude")
      .join("claude_desktop_config.json")
  })
}

#[cfg(target_os = "windows")]
fn claude_desktop_config_path() -> Option<PathBuf> {
  std::env::var("APPDATA").ok().map(|a| {
    PathBuf::from(a)
      .join("Claude")
      .join("claude_desktop_config.json")
  })
}

#[cfg(target_os = "linux")]
fn claude_desktop_config_path() -> Option<PathBuf> {
  let base = std::env::var("XDG_CONFIG_HOME")
    .ok()
    .map(PathBuf::from)
    .or_else(|| home().map(|h| h.join(".config")))?;
  Some(base.join("Claude").join("claude_desktop_config.json"))
}

const AGENT_SPECS: &[AgentSpec] = &[
  AgentSpec {
    id: "claude-desktop",
    display_name: "Claude Desktop",
    category: AgentCategory::DesktopApp,
    config_key: "mcpServers",
    format: ConfigFormat::Json,
  },
  AgentSpec {
    id: "claude-code",
    display_name: "Claude Code",
    category: AgentCategory::Cli,
    config_key: "mcpServers",
    format: ConfigFormat::Json,
  },
  AgentSpec {
    id: "cursor",
    display_name: "Cursor",
    category: AgentCategory::Editor,
    config_key: "mcpServers",
    format: ConfigFormat::Json,
  },
  AgentSpec {
    id: "vscode",
    display_name: "VS Code",
    category: AgentCategory::Editor,
    config_key: "servers",
    format: ConfigFormat::Json,
  },
  AgentSpec {
    id: "zed",
    display_name: "Zed",
    category: AgentCategory::Editor,
    config_key: "context_servers",
    format: ConfigFormat::Json,
  },
  AgentSpec {
    id: "cline-cli",
    display_name: "Cline CLI",
    category: AgentCategory::Cli,
    config_key: "mcpServers",
    format: ConfigFormat::Json,
  },
  AgentSpec {
    id: "cline",
    display_name: "Cline VSCode",
    category: AgentCategory::EditorExt,
    config_key: "mcpServers",
    format: ConfigFormat::Json,
  },
  AgentSpec {
    id: "codex",
    display_name: "Codex",
    category: AgentCategory::Cli,
    config_key: "mcp_servers",
    format: ConfigFormat::Toml,
  },
  AgentSpec {
    id: "gemini-cli",
    display_name: "Gemini CLI",
    category: AgentCategory::Cli,
    config_key: "mcpServers",
    format: ConfigFormat::Json,
  },
  AgentSpec {
    id: "github-copilot-cli",
    display_name: "GitHub Copilot CLI",
    category: AgentCategory::Cli,
    config_key: "mcpServers",
    format: ConfigFormat::Json,
  },
  AgentSpec {
    id: "goose",
    display_name: "Goose",
    category: AgentCategory::Cli,
    config_key: "extensions",
    format: ConfigFormat::Yaml,
  },
  AgentSpec {
    id: "antigravity",
    display_name: "Antigravity",
    category: AgentCategory::DesktopApp,
    config_key: "mcpServers",
    format: ConfigFormat::Json,
  },
  AgentSpec {
    id: "opencode",
    display_name: "OpenCode",
    category: AgentCategory::Cli,
    config_key: "mcp",
    format: ConfigFormat::Json,
  },
  AgentSpec {
    id: "mcporter",
    display_name: "MCPorter",
    category: AgentCategory::Cli,
    config_key: "mcpServers",
    format: ConfigFormat::Json,
  },
];

fn spec_for(agent_id: &str) -> Option<&'static AgentSpec> {
  AGENT_SPECS.iter().find(|s| s.id == agent_id)
}

fn detect_agent_directory(agent_id: &str) -> bool {
  // Mirrors add-mcp's `detectGlobalInstall` checks — typically the immediate
  // parent of the config file. Used only for UI annotation; install/uninstall
  // always operates on the resolved config path.
  let Some(h) = home() else {
    return false;
  };
  match agent_id {
    "antigravity" => h.join(".gemini").exists(),
    "cline" => config_path_for("cline")
      .and_then(|p| p.parent().map(|d| d.exists()))
      .unwrap_or(false),
    "cline-cli" => config_path_for("cline-cli")
      .and_then(|p| p.parent().map(|d| d.exists()))
      .unwrap_or(false),
    "claude-code" => h.join(".claude").exists(),
    "claude-desktop" => claude_desktop_config_path()
      .and_then(|p| p.parent().map(|d| d.exists()))
      .unwrap_or(false),
    "codex" => h.join(".codex").exists(),
    "cursor" => h.join(".cursor").exists(),
    "gemini-cli" => h.join(".gemini").exists(),
    "github-copilot-cli" => config_path_for("github-copilot-cli")
      .and_then(|p| p.parent().map(|d| d.exists()))
      .unwrap_or(false),
    "goose" => goose_config_path().is_some_and(|p| p.exists()),
    "mcporter" => h.join(".mcporter").exists(),
    "opencode" => h.join(".config").join("opencode").exists(),
    "vscode" => vscode_user_dir().is_some_and(|d| d.exists()),
    "zed" => zed_config_dir().is_some_and(|d| d.exists()),
    _ => false,
  }
}

/// Transform the donut-browser HTTP server config into the per-agent shape.
/// All agents speak HTTP except Claude Desktop, which uses a node stdio bridge
/// (handled by the extension installer in lib.rs).
fn transform_remote_config(agent_id: &str, url: &str) -> serde_json::Value {
  use serde_json::json;
  match agent_id {
    "zed" => json!({ "source": "custom", "type": "http", "url": url }),
    "opencode" => json!({ "type": "remote", "url": url, "enabled": true }),
    "antigravity" => json!({ "serverUrl": url }),
    "cursor" => json!({ "url": url }),
    "cline" | "cline-cli" => json!({
      "url": url,
      "type": "streamableHttp",
      "disabled": false,
    }),
    "codex" => json!({ "type": "http", "url": url }),
    "github-copilot-cli" => json!({ "type": "http", "url": url, "tools": ["*"] }),
    "goose" => json!({
      "name": SERVER_NAME,
      "description": "",
      "type": "streamable_http",
      "uri": url,
      "headers": {},
      "enabled": true,
      "timeout": 300,
    }),
    "vscode" => json!({ "type": "http", "url": url }),
    // claude-code, claude-desktop, gemini-cli, mcporter — passthrough
    _ => json!({ "type": "http", "url": url }),
  }
}

/// Detect whether a server config object looks like our donut-browser HTTP
/// endpoint by URL prefix. Matches across the various per-agent key shapes
/// (`url`, `uri`, `serverUrl`).
fn config_matches_donut(value: &serde_json::Value) -> bool {
  for key in ["url", "uri", "serverUrl"] {
    if let Some(s) = value.get(key).and_then(|v| v.as_str()) {
      if s.contains("/mcp/")
        && (s.starts_with("http://127.0.0.1") || s.starts_with("http://localhost"))
      {
        return true;
      }
    }
  }
  false
}

fn read_value(path: &Path, format: ConfigFormat) -> serde_json::Value {
  let Ok(content) = fs::read_to_string(path) else {
    return serde_json::Value::Null;
  };
  match format {
    ConfigFormat::Json => serde_json::from_str(&content).unwrap_or(serde_json::Value::Null),
    ConfigFormat::Toml => toml::from_str::<toml::Value>(&content)
      .ok()
      .and_then(|t| serde_json::to_value(t).ok())
      .unwrap_or(serde_json::Value::Null),
    ConfigFormat::Yaml => serde_yaml::from_str::<serde_yaml::Value>(&content)
      .ok()
      .and_then(|y| serde_json::to_value(y).ok())
      .unwrap_or(serde_json::Value::Null),
  }
}

fn write_value(path: &Path, value: &serde_json::Value, format: ConfigFormat) -> Result<(), String> {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).map_err(|e| format!("Failed to create config dir: {e}"))?;
  }
  let content = match format {
    ConfigFormat::Json => {
      serde_json::to_string_pretty(value).map_err(|e| format!("Failed to serialize JSON: {e}"))?
    }
    ConfigFormat::Toml => {
      let toml_val: toml::Value = serde_json::from_value(value.clone())
        .map_err(|e| format!("Failed to convert to TOML: {e}"))?;
      toml::to_string_pretty(&toml_val).map_err(|e| format!("Failed to serialize TOML: {e}"))?
    }
    ConfigFormat::Yaml => {
      let yaml_val: serde_yaml::Value = serde_yaml::from_str(
        &serde_json::to_string(value).map_err(|e| format!("Failed to serialize: {e}"))?,
      )
      .map_err(|e| format!("Failed to convert to YAML: {e}"))?;
      serde_yaml::to_string(&yaml_val).map_err(|e| format!("Failed to serialize YAML: {e}"))?
    }
  };
  fs::write(path, content).map_err(|e| format!("Failed to write config: {e}"))?;
  Ok(())
}

/// Navigate `config_key` (dot notation), creating object literals at each
/// missing level. Returns a mutable reference to the bottom container so the
/// caller can set/remove server entries.
fn ensure_nested_object<'a>(
  root: &'a mut serde_json::Value,
  config_key: &str,
) -> &'a mut serde_json::Map<String, serde_json::Value> {
  if !root.is_object() {
    *root = serde_json::Value::Object(serde_json::Map::new());
  }
  let mut current = root.as_object_mut().expect("just set to object");
  let parts: Vec<&str> = config_key.split('.').collect();
  for part in &parts {
    let entry = current
      .entry(part.to_string())
      .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if !entry.is_object() {
      *entry = serde_json::Value::Object(serde_json::Map::new());
    }
    current = entry.as_object_mut().expect("just ensured object");
  }
  current
}

fn nested_object<'a>(
  root: &'a serde_json::Value,
  config_key: &str,
) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
  let mut current = root.as_object()?;
  for part in config_key.split('.') {
    current = current.get(part)?.as_object()?;
  }
  Some(current)
}

fn is_generic_agent_connected(agent_id: &str) -> bool {
  let Some(spec) = spec_for(agent_id) else {
    return false;
  };
  let Some(path) = config_path_for(agent_id) else {
    return false;
  };
  if !path.exists() {
    return false;
  }
  let root = read_value(&path, spec.format);
  let Some(servers) = nested_object(&root, spec.config_key) else {
    return false;
  };
  if let Some(entry) = servers.get(SERVER_NAME) {
    return config_matches_donut(entry);
  }
  servers.values().any(config_matches_donut)
}

/// Install or remove the donut-browser entry from a generic agent. Returns
/// `true` if a write happened. Callers handle higher-level dispatch (Claude
/// Desktop extension setup, Claude Code CLI invocation).
pub fn install_generic(agent_id: &str, url: &str) -> Result<(), String> {
  let spec = spec_for(agent_id).ok_or_else(|| format!("Unknown agent: {agent_id}"))?;
  let path = config_path_for(agent_id)
    .ok_or_else(|| format!("Unable to resolve config path for {agent_id}"))?;

  let mut root = if path.exists() {
    read_value(&path, spec.format)
  } else {
    serde_json::Value::Object(serde_json::Map::new())
  };
  if !root.is_object() {
    root = serde_json::Value::Object(serde_json::Map::new());
  }

  let container = ensure_nested_object(&mut root, spec.config_key);
  container.insert(
    SERVER_NAME.to_string(),
    transform_remote_config(agent_id, url),
  );

  write_value(&path, &root, spec.format)
}

pub fn uninstall_generic(agent_id: &str) -> Result<(), String> {
  let spec = spec_for(agent_id).ok_or_else(|| format!("Unknown agent: {agent_id}"))?;
  let Some(path) = config_path_for(agent_id) else {
    return Ok(());
  };
  if !path.exists() {
    return Ok(());
  }

  let mut root = read_value(&path, spec.format);
  if !root.is_object() {
    return Ok(());
  }

  let container = ensure_nested_object(&mut root, spec.config_key);
  container.remove(SERVER_NAME);

  write_value(&path, &root, spec.format)
}

pub fn list_agents_with_status(connected_overrides: &[(&str, bool)]) -> Vec<McpAgentInfo> {
  AGENT_SPECS
    .iter()
    .map(|spec| {
      let connected = connected_overrides
        .iter()
        .find(|(id, _)| *id == spec.id)
        .map(|(_, c)| *c)
        .unwrap_or_else(|| is_generic_agent_connected(spec.id));
      McpAgentInfo {
        id: spec.id.to_string(),
        display_name: spec.display_name.to_string(),
        category: spec.category,
        connected,
        detected: detect_agent_directory(spec.id),
      }
    })
    .collect()
}

pub fn agent_exists(agent_id: &str) -> bool {
  spec_for(agent_id).is_some()
}
