//! MCP client installer: writes the Donut Browser server entry into the global
//! config of twenty AI clients and reads it back for the Integrations page.
//!
//! Two endpoints exist. The local loopback server carries its token in the
//! URL; the remote endpoint under `CLOUD_API_URL` needs a bearer credential
//! and is only offered to accounts with remote control. `McpTarget` holds
//! either, and `server_entry` decides per client where the credential goes:
//! most clients
//! take a `headers` map, Codex calls it `http_headers`, and fx refuses a
//! literal header and reads the token from `DONUT_MCP_TOKEN` instead.
//!
//! Every write edits the user's file in place. JSON goes through a concrete
//! syntax tree so comments, key order, indentation and trailing commas survive
//! (`~/.claude.json` is a large file with many unrelated keys; VS Code, Zed,
//! Gemini, OpenCode, Kilo and MCPorter files carry comments by design). TOML
//! goes through `toml_edit`, which keeps comments and table order. YAML is
//! rewritten from an order-preserving mapping; Goose tolerates that. A file
//! that does not parse aborts the operation with the file untouched: the old
//! installer treated a parse failure as an empty file and wiped configs that
//! merely had a comment in them.
//!
//! Claude Desktop has no HTTP entry in its config file; lib.rs installs a
//! local extension bundle with a node bridge instead. This module only
//! resolves its directory.

use jsonc_parser::cst::{CstInputValue, CstObject, CstRootNode};
use jsonc_parser::ParseOptions;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table};

pub const SERVER_NAME: &str = "donut-browser";
const REMOTE_MCP_PATH: &str = "/api/mcp";
/// fx rejects a literal `Authorization` header in its config and reads the
/// bearer from an environment variable named in the entry instead.
pub const FX_TOKEN_ENV: &str = "DONUT_MCP_TOKEN";

pub fn remote_mcp_url() -> String {
  format!("{}{REMOTE_MCP_PATH}", crate::cloud_auth::CLOUD_API_URL)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpEndpoint {
  Remote,
  Local,
}

impl McpEndpoint {
  pub fn parse(target: &str) -> Option<Self> {
    match target {
      "remote" => Some(Self::Remote),
      "local" => Some(Self::Local),
      _ => None,
    }
  }
}

/// What gets written into a client: the URL and, for the remote endpoint,
/// the bearer credential. The local server authenticates through the token
/// in its URL, so it carries no bearer.
#[derive(Debug, Clone)]
pub struct McpTarget {
  pub url: String,
  pub bearer: Option<String>,
}

impl McpTarget {
  pub fn remote(key: String) -> Self {
    Self {
      url: remote_mcp_url(),
      bearer: Some(key),
    }
  }

  // TODO(local-mcp-removal): local MCP is removed; nothing in production builds
  // a local target any more (only tests still exercise the shape). Delete this
  // together with the loopback tombstone and the `Local` endpoint variant once
  // the deprecation period ends.
  #[allow(dead_code)]
  pub fn local(url: String) -> Self {
    Self { url, bearer: None }
  }

  fn authorization(&self) -> Option<String> {
    self.bearer.as_ref().map(|key| format!("Bearer {key}"))
  }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum AgentCategory {
  DesktopApp,
  Cli,
  Editor,
  EditorExt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
  /// Top-level key under which the client keeps its server map.
  config_key: &'static str,
  format: ConfigFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentStatus {
  pub connected: bool,
  pub endpoint: Option<McpEndpoint>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpAgentInfo {
  pub id: String,
  pub display_name: String,
  pub category: AgentCategory,
  pub connected: bool,
  /// True when the client itself appears to be installed (its config
  /// directory exists), whether or not Donut is configured in it.
  pub detected: bool,
  /// Which Donut endpoint the client's entry points at, when connected.
  pub endpoint: Option<McpEndpoint>,
  /// Set for clients that read the bearer from an environment variable
  /// instead of the config file, so the UI can tell the user to export it.
  pub token_env: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
  MacOs,
  Linux,
  Windows,
}

impl Platform {
  pub fn current() -> Self {
    if cfg!(target_os = "windows") {
      Self::Windows
    } else if cfg!(target_os = "macos") {
      Self::MacOs
    } else {
      Self::Linux
    }
  }
}

/// Everything path resolution reads from the environment, captured once.
/// Tests build one over a temp directory instead of mutating the process
/// environment, which parallel tests would race on.
#[derive(Debug, Clone)]
pub struct AgentEnv {
  pub platform: Platform,
  pub home: PathBuf,
  pub appdata: Option<PathBuf>,
  pub xdg_config_home: Option<PathBuf>,
  pub codex_home: Option<PathBuf>,
  pub grok_home: Option<PathBuf>,
  pub kimi_code_home: Option<PathBuf>,
  pub cline_dir: Option<PathBuf>,
}

fn env_path(name: &str) -> Option<PathBuf> {
  std::env::var_os(name)
    .filter(|value| !value.is_empty())
    .map(PathBuf::from)
}

impl AgentEnv {
  pub fn from_process() -> Option<Self> {
    Some(Self {
      platform: Platform::current(),
      home: dirs::home_dir()?,
      appdata: env_path("APPDATA"),
      xdg_config_home: env_path("XDG_CONFIG_HOME"),
      codex_home: env_path("CODEX_HOME"),
      grok_home: env_path("GROK_HOME"),
      kimi_code_home: env_path("KIMI_CODE_HOME"),
      cline_dir: env_path("CLINE_DIR"),
    })
  }

  /// `$XDG_CONFIG_HOME`, else `~/.config`, on every platform: the clients that
  /// follow the XDG layout do so on Windows too.
  fn config_home(&self) -> PathBuf {
    self
      .xdg_config_home
      .clone()
      .unwrap_or_else(|| self.home.join(".config"))
  }

  fn appdata(&self) -> PathBuf {
    self
      .appdata
      .clone()
      .unwrap_or_else(|| self.home.join("AppData").join("Roaming"))
  }

  /// Where desktop apps keep per-user data: Application Support on macOS,
  /// roaming AppData on Windows, the XDG config dir on Linux.
  fn app_support(&self) -> PathBuf {
    match self.platform {
      Platform::MacOs => self.home.join("Library").join("Application Support"),
      Platform::Windows => self.appdata(),
      Platform::Linux => self.config_home(),
    }
  }

  fn vscode_user_dir(&self) -> PathBuf {
    self.app_support().join("Code").join("User")
  }

  pub fn claude_desktop_dir(&self) -> PathBuf {
    self.app_support().join("Claude")
  }

  fn zed_config_dir(&self) -> PathBuf {
    match self.platform {
      // Zed's Linux directory is lower-case; the other two keep the app name.
      Platform::Linux => self.config_home().join("zed"),
      _ => self.app_support().join("Zed"),
    }
  }

  fn goose_config_path(&self) -> PathBuf {
    match self.platform {
      Platform::Windows => self
        .appdata()
        .join("Block")
        .join("goose")
        .join("config")
        .join("config.yaml"),
      // Goose on macOS reads ~/.config even when XDG_CONFIG_HOME is set.
      Platform::MacOs => self.home.join(".config").join("goose").join("config.yaml"),
      Platform::Linux => self.config_home().join("goose").join("config.yaml"),
    }
  }

  fn codex_home(&self) -> PathBuf {
    self
      .codex_home
      .clone()
      .unwrap_or_else(|| self.home.join(".codex"))
  }

  fn grok_home(&self) -> PathBuf {
    self
      .grok_home
      .clone()
      .unwrap_or_else(|| self.home.join(".grok"))
  }

  fn kimi_code_home(&self) -> PathBuf {
    self
      .kimi_code_home
      .clone()
      .unwrap_or_else(|| self.home.join(".kimi-code"))
  }

  fn cline_dir(&self) -> PathBuf {
    self
      .cline_dir
      .clone()
      .unwrap_or_else(|| self.home.join(".cline"))
  }

  fn kilo_config_dir(&self) -> PathBuf {
    self.config_home().join("kilo")
  }
}

/// The first candidate that already exists, else `default`, which the install
/// will create. Clients that accept both `.json` and `.jsonc` must not end up
/// with two competing files.
fn first_existing(candidates: &[PathBuf], default: PathBuf) -> PathBuf {
  candidates
    .iter()
    .find(|candidate| candidate.exists())
    .cloned()
    .unwrap_or(default)
}

fn config_path(env: &AgentEnv, agent_id: &str) -> Option<PathBuf> {
  let home = &env.home;
  let path = match agent_id {
    "antigravity" => home.join(".gemini").join("config").join("mcp_config.json"),
    "cline" => env
      .vscode_user_dir()
      .join("globalStorage")
      .join("saoudrizwan.claude-dev")
      .join("settings")
      .join("cline_mcp_settings.json"),
    "cline-cli" => env
      .cline_dir()
      .join("data")
      .join("settings")
      .join("cline_mcp_settings.json"),
    "claude-code" => home.join(".claude.json"),
    "claude-desktop" => env.claude_desktop_dir().join("claude_desktop_config.json"),
    "codex" => env.codex_home().join("config.toml"),
    "cursor" => home.join(".cursor").join("mcp.json"),
    "fx" => home.join(".fx").join("mcp.json"),
    "gemini-cli" => home.join(".gemini").join("settings.json"),
    "goose" => env.goose_config_path(),
    // Copilot joins XDG_CONFIG_HOME directly rather than a subdirectory of it.
    "github-copilot-cli" => env
      .xdg_config_home
      .clone()
      .unwrap_or_else(|| home.join(".copilot"))
      .join("mcp-config.json"),
    "grok-build" => env.grok_home().join("config.toml"),
    "kilo-code" => {
      let dir = env.kilo_config_dir();
      first_existing(
        &[dir.join("kilo.jsonc"), dir.join("kilo.json")],
        dir.join("kilo.json"),
      )
    }
    "kimi-code" => env.kimi_code_home().join("mcp.json"),
    "kiro-cli" => home.join(".kiro").join("settings").join("mcp.json"),
    "mcporter" => {
      let dir = home.join(".mcporter");
      first_existing(
        &[dir.join("mcporter.json"), dir.join("mcporter.jsonc")],
        dir.join("mcporter.json"),
      )
    }
    "opencode" => {
      // OpenCode reads ~/.config on every platform, with no XDG override, and
      // its own docs prefer the .jsonc spelling for new files.
      let dir = home.join(".config").join("opencode");
      first_existing(
        &[dir.join("opencode.jsonc"), dir.join("opencode.json")],
        dir.join("opencode.jsonc"),
      )
    }
    "vscode" => env.vscode_user_dir().join("mcp.json"),
    "windsurf" => home
      .join(".codeium")
      .join("windsurf")
      .join("mcp_config.json"),
    "zed" => env.zed_config_dir().join("settings.json"),
    _ => return None,
  };
  Some(path)
}

/// Whether the client itself looks installed. A UI annotation only: install
/// and uninstall always operate on the resolved config path.
fn detected(env: &AgentEnv, agent_id: &str) -> bool {
  let home = &env.home;
  let parent_exists = || {
    config_path(env, agent_id)
      .and_then(|path| path.parent().map(Path::exists))
      .unwrap_or(false)
  };
  match agent_id {
    "antigravity" => home.join(".gemini").join("config").exists(),
    "cline" | "cline-cli" | "github-copilot-cli" => parent_exists(),
    "claude-code" => home.join(".claude").exists(),
    "claude-desktop" => env.claude_desktop_dir().exists(),
    // Codex detection looks at the default directory even when CODEX_HOME
    // points elsewhere; the install still honours the override.
    "codex" => home.join(".codex").exists(),
    "cursor" => home.join(".cursor").exists(),
    "fx" => home.join(".fx").exists(),
    "gemini-cli" => home.join(".gemini").exists(),
    "goose" => env.goose_config_path().exists(),
    "grok-build" => env.grok_home().exists(),
    "kilo-code" => {
      env.kilo_config_dir().exists()
        || env
          .vscode_user_dir()
          .join("globalStorage")
          .join("kilocode.kilo-code")
          .exists()
    }
    "kimi-code" => env.kimi_code_home().exists(),
    "kiro-cli" => home.join(".kiro").exists(),
    "mcporter" => home.join(".mcporter").exists(),
    "opencode" => home.join(".config").join("opencode").exists(),
    "vscode" => env.vscode_user_dir().exists(),
    "windsurf" => home.join(".codeium").join("windsurf").exists(),
    "zed" => env.zed_config_dir().exists(),
    _ => false,
  }
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
    id: "windsurf",
    display_name: "Windsurf",
    category: AgentCategory::Editor,
    config_key: "mcpServers",
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
    id: "fx",
    display_name: "fx",
    category: AgentCategory::Cli,
    config_key: "mcp",
    format: ConfigFormat::Json,
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
    id: "grok-build",
    display_name: "Grok Build",
    category: AgentCategory::Cli,
    config_key: "mcp_servers",
    format: ConfigFormat::Toml,
  },
  AgentSpec {
    id: "antigravity",
    display_name: "Antigravity",
    category: AgentCategory::DesktopApp,
    config_key: "mcpServers",
    format: ConfigFormat::Json,
  },
  AgentSpec {
    id: "kilo-code",
    display_name: "Kilo Code",
    category: AgentCategory::Cli,
    config_key: "mcp",
    format: ConfigFormat::Json,
  },
  AgentSpec {
    id: "kimi-code",
    display_name: "Kimi Code",
    category: AgentCategory::Cli,
    config_key: "mcpServers",
    format: ConfigFormat::Json,
  },
  AgentSpec {
    id: "kiro-cli",
    display_name: "Kiro CLI",
    category: AgentCategory::Cli,
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
  AGENT_SPECS.iter().find(|spec| spec.id == agent_id)
}

fn token_env_for(agent_id: &str) -> Option<String> {
  (agent_id == "fx").then(|| FX_TOKEN_ENV.to_string())
}

/// A server entry with its keys in the order the client's own docs show them.
/// `serde_json::Map` sorts keys, so the shape stays a list until written.
type Entry = Vec<(&'static str, serde_json::Value)>;

/// The per-client shape of the Donut entry, including where the credential
/// goes. Every install replaces the entry wholesale so stale keys from an
/// earlier shape never linger.
fn server_entry(agent_id: &str, target: &McpTarget) -> Entry {
  use serde_json::json;
  let url = json!(target.url);
  let headers = target
    .authorization()
    .map(|value| json!({ "Authorization": value }));
  let mut entry: Entry = match agent_id {
    "antigravity" | "windsurf" => vec![("serverUrl", url)],
    "cursor" | "kiro-cli" | "grok-build" => vec![("url", url)],
    "cline" | "cline-cli" => vec![
      ("url", url),
      ("type", json!("streamableHttp")),
      ("disabled", json!(false)),
    ],
    "kimi-code" => vec![("transport", json!("http")), ("url", url)],
    "github-copilot-cli" => vec![
      ("type", json!("http")),
      ("url", url),
      ("tools", json!(["*"])),
    ],
    "zed" => vec![
      ("source", json!("custom")),
      ("type", json!("http")),
      ("url", url),
    ],
    "opencode" | "kilo-code" => vec![
      ("type", json!("remote")),
      ("url", url),
      ("enabled", json!(true)),
    ],
    "goose" => vec![
      ("name", json!(SERVER_NAME)),
      ("description", json!("")),
      ("type", json!("streamable_http")),
      ("uri", url),
    ],
    "fx" => vec![
      ("type", json!("http")),
      ("url", url),
      ("enabled", json!(true)),
    ],
    // claude-code, codex, gemini-cli, mcporter, vscode: the standard
    // streamable HTTP shape.
    _ => vec![("type", json!("http")), ("url", url)],
  };
  match agent_id {
    "codex" => {
      if let Some(headers) = headers {
        entry.push(("http_headers", headers));
      }
    }
    "fx" => {
      if target.bearer.is_some() {
        entry.push(("bearer_token_env", json!(FX_TOKEN_ENV)));
      }
    }
    // Zed and Goose document the header map as part of the shape, so it is
    // written even when there is nothing to put in it.
    "zed" | "goose" => entry.push(("headers", headers.unwrap_or_else(|| json!({})))),
    _ => {
      if let Some(headers) = headers {
        entry.push(("headers", headers));
      }
    }
  }
  if agent_id == "goose" {
    entry.push(("enabled", json!(true)));
    entry.push(("timeout", json!(300)));
  }
  entry
}

/// Which Donut endpoint a URL points at, if any. The remote URL is matched
/// exactly (a trailing slash tolerated); the loopback form is any plain-http
/// `127.0.0.1` or `localhost` origin whose path starts with `/mcp`.
pub fn endpoint_of_url(url: &str) -> Option<McpEndpoint> {
  let url = url.trim();
  if url.trim_end_matches('/') == remote_mcp_url().trim_end_matches('/') {
    return Some(McpEndpoint::Remote);
  }
  let rest = url.strip_prefix("http://")?;
  let (authority, path) = match rest.find('/') {
    Some(index) => rest.split_at(index),
    None => (rest, ""),
  };
  let host = authority.split(':').next().unwrap_or("");
  let loopback = host == "127.0.0.1" || host == "localhost";
  let mcp_path = path == "/mcp" || path.starts_with("/mcp/");
  (loopback && mcp_path).then_some(McpEndpoint::Local)
}

fn endpoint_of_entry(value: &serde_json::Value) -> Option<McpEndpoint> {
  ["url", "uri", "serverUrl"].iter().find_map(|key| {
    value
      .get(key)
      .and_then(|v| v.as_str())
      .and_then(endpoint_of_url)
  })
}

/// Ours by name, or ours by URL under any name: what detection counts as
/// connected is exactly what removal deletes.
fn is_donut_entry(name: &str, value: &serde_json::Value) -> bool {
  name == SERVER_NAME || endpoint_of_entry(value).is_some()
}

fn json_to_cst(value: &serde_json::Value) -> CstInputValue {
  match value {
    serde_json::Value::Null => CstInputValue::Null,
    serde_json::Value::Bool(b) => CstInputValue::Bool(*b),
    serde_json::Value::Number(n) => CstInputValue::Number(n.to_string()),
    serde_json::Value::String(s) => CstInputValue::String(s.clone()),
    serde_json::Value::Array(items) => {
      CstInputValue::Array(items.iter().map(json_to_cst).collect())
    }
    serde_json::Value::Object(map) => CstInputValue::Object(
      map
        .iter()
        .map(|(key, value)| (key.clone(), json_to_cst(value)))
        .collect(),
    ),
  }
}

fn entry_to_cst(entry: &Entry) -> CstInputValue {
  CstInputValue::Object(
    entry
      .iter()
      .map(|(key, value)| (key.to_string(), json_to_cst(value)))
      .collect(),
  )
}

fn json_to_toml(value: &serde_json::Value) -> Result<toml_edit::Value, String> {
  Ok(match value {
    serde_json::Value::Null => return Err("TOML has no null value".to_string()),
    serde_json::Value::Bool(b) => toml_edit::Value::from(*b),
    serde_json::Value::Number(n) => match (n.as_i64(), n.as_f64()) {
      (Some(i), _) => toml_edit::Value::from(i),
      (None, Some(f)) => toml_edit::Value::from(f),
      (None, None) => return Err(format!("{n} does not fit a TOML number")),
    },
    serde_json::Value::String(s) => toml_edit::Value::from(s.as_str()),
    serde_json::Value::Array(items) => toml_edit::Value::Array(
      items
        .iter()
        .map(json_to_toml)
        .collect::<Result<toml_edit::Array, String>>()?,
    ),
    serde_json::Value::Object(map) => toml_edit::Value::InlineTable(
      map
        .iter()
        .map(|(key, value)| json_to_toml(value).map(|v| (key.clone(), v)))
        .collect::<Result<toml_edit::InlineTable, String>>()?,
    ),
  })
}

fn entry_to_toml(entry: &Entry) -> Result<Table, String> {
  let mut table = Table::new();
  for (key, value) in entry {
    table.insert(key, Item::Value(json_to_toml(value)?));
  }
  Ok(table)
}

fn toml_value_to_json(value: &toml_edit::Value) -> serde_json::Value {
  use serde_json::json;
  match value {
    toml_edit::Value::String(s) => json!(s.value()),
    toml_edit::Value::Integer(i) => json!(*i.value()),
    toml_edit::Value::Float(f) => json!(*f.value()),
    toml_edit::Value::Boolean(b) => json!(*b.value()),
    toml_edit::Value::Datetime(d) => json!(d.value().to_string()),
    toml_edit::Value::Array(items) => {
      serde_json::Value::Array(items.iter().map(toml_value_to_json).collect())
    }
    toml_edit::Value::InlineTable(table) => serde_json::Value::Object(
      table
        .iter()
        .map(|(key, value)| (key.to_string(), toml_value_to_json(value)))
        .collect(),
    ),
  }
}

fn toml_item_to_json(item: &Item) -> serde_json::Value {
  match item {
    Item::None => serde_json::Value::Null,
    Item::Value(value) => toml_value_to_json(value),
    Item::Table(table) => serde_json::Value::Object(
      table
        .iter()
        .map(|(key, item)| (key.to_string(), toml_item_to_json(item)))
        .collect(),
    ),
    Item::ArrayOfTables(tables) => serde_json::Value::Array(
      tables
        .iter()
        .map(|table| toml_item_to_json(&Item::Table(table.clone())))
        .collect(),
    ),
  }
}

fn entry_to_yaml(entry: &Entry) -> Result<serde_yaml::Value, String> {
  let mut mapping = serde_yaml::Mapping::new();
  for (key, value) in entry {
    let yaml =
      serde_yaml::to_value(value).map_err(|e| format!("Failed to convert to YAML: {e}"))?;
    mapping.insert(serde_yaml::Value::String(key.to_string()), yaml);
  }
  Ok(serde_yaml::Value::Mapping(mapping))
}

/// A parsed config file that can be edited without disturbing what the user
/// wrote around the Donut entry.
enum Document {
  Json(CstRootNode),
  Toml(DocumentMut),
  Yaml(serde_yaml::Value),
}

impl Document {
  fn parse(content: &str, format: ConfigFormat) -> Result<Self, String> {
    Ok(match format {
      ConfigFormat::Json => Self::Json(
        CstRootNode::parse(content, &ParseOptions::default()).map_err(|e| e.to_string())?,
      ),
      ConfigFormat::Toml => Self::Toml(content.parse::<DocumentMut>().map_err(|e| e.to_string())?),
      ConfigFormat::Yaml => {
        let value = if content.trim().is_empty() {
          serde_yaml::Value::Null
        } else {
          serde_yaml::from_str(content).map_err(|e| e.to_string())?
        };
        Self::Yaml(value)
      }
    })
  }

  fn json_servers(root: &CstRootNode, key: &str) -> Option<CstObject> {
    root.object_value()?.object_value(key)
  }

  /// Every entry under the server map, as plain JSON for inspection.
  fn entries(&self, key: &str) -> Vec<(String, serde_json::Value)> {
    match self {
      Self::Json(root) => Self::json_servers(root, key)
        .map(|servers| {
          servers
            .properties()
            .into_iter()
            .filter_map(|prop| {
              let name = prop.name()?.decoded_value().ok()?;
              let value = prop.to_serde_value()?;
              Some((name, value))
            })
            .collect()
        })
        .unwrap_or_default(),
      Self::Toml(doc) => doc
        .get(key)
        .and_then(Item::as_table_like)
        .map(|servers| {
          servers
            .iter()
            .map(|(name, item)| (name.to_string(), toml_item_to_json(item)))
            .collect()
        })
        .unwrap_or_default(),
      Self::Yaml(root) => root
        .get(key)
        .and_then(serde_yaml::Value::as_mapping)
        .map(|servers| {
          servers
            .iter()
            .filter_map(|(name, value)| {
              let name = name.as_str()?.to_string();
              let value = serde_json::to_value(value).ok()?;
              Some((name, value))
            })
            .collect()
        })
        .unwrap_or_default(),
    }
  }

  /// Write the Donut entry, replacing any entry of that name wholesale. A root
  /// or server map that exists but is not an object is an error rather than
  /// something to overwrite.
  fn set_entry(&mut self, key: &str, entry: &Entry) -> Result<(), String> {
    match self {
      Self::Json(root) => {
        let root_object = root
          .object_value_or_create()
          .ok_or("the file's top level is not a JSON object")?;
        let servers = root_object
          .object_value_or_create(key)
          .ok_or_else(|| format!("\"{key}\" is not a JSON object"))?;
        match servers.get(SERVER_NAME) {
          Some(prop) => prop.set_value(entry_to_cst(entry)),
          None => {
            servers.append(SERVER_NAME, entry_to_cst(entry));
          }
        }
      }
      Self::Toml(doc) => {
        let servers = doc.entry(key).or_insert_with(|| {
          // Implicit: the entry renders as [mcp_servers.donut-browser] with no
          // bare [mcp_servers] header above it, the way Codex writes it.
          let mut table = Table::new();
          table.set_implicit(true);
          Item::Table(table)
        });
        let servers = servers
          .as_table_like_mut()
          .ok_or_else(|| format!("\"{key}\" is not a TOML table"))?;
        servers.insert(SERVER_NAME, Item::Table(entry_to_toml(entry)?));
      }
      Self::Yaml(root) => {
        if root.is_null() {
          *root = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        }
        let root_map = root
          .as_mapping_mut()
          .ok_or("the file's top level is not a YAML mapping")?;
        let servers = root_map
          .entry(serde_yaml::Value::String(key.to_string()))
          .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
        if servers.is_null() {
          *servers = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        }
        servers
          .as_mapping_mut()
          .ok_or_else(|| format!("\"{key}\" is not a YAML mapping"))?
          .insert(
            serde_yaml::Value::String(SERVER_NAME.to_string()),
            entry_to_yaml(entry)?,
          );
      }
    }
    Ok(())
  }

  fn remove_entries(&mut self, key: &str, names: &[String]) {
    match self {
      Self::Json(root) => {
        if let Some(servers) = Self::json_servers(root, key) {
          for name in names {
            if let Some(prop) = servers.get(name) {
              prop.remove();
            }
          }
        }
      }
      Self::Toml(doc) => {
        if let Some(servers) = doc.get_mut(key).and_then(Item::as_table_like_mut) {
          for name in names {
            servers.remove(name);
          }
        }
      }
      Self::Yaml(root) => {
        if let Some(servers) = root
          .get_mut(key)
          .and_then(serde_yaml::Value::as_mapping_mut)
        {
          for name in names {
            servers.remove(name.as_str());
          }
        }
      }
    }
  }

  fn to_text(&self) -> Result<String, String> {
    match self {
      Self::Json(root) => Ok(root.to_string()),
      Self::Toml(doc) => Ok(doc.to_string()),
      Self::Yaml(root) => {
        serde_yaml::to_string(root).map_err(|e| format!("Failed to serialize YAML: {e}"))
      }
    }
  }
}

fn read_document(path: &Path, format: ConfigFormat) -> Result<(Document, bool), String> {
  let content = if path.exists() {
    fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?
  } else {
    String::new()
  };
  let document = Document::parse(&content, format).map_err(|e| {
    format!(
      "{} could not be parsed, so it was left untouched: {e}",
      path.display()
    )
  })?;
  Ok((document, content.is_empty()))
}

/// Replace the file through a sibling temp file and rename, so a crash mid
/// write never leaves half of `~/.claude.json` behind. An existing file keeps
/// its permission bits. A new one is owner-only when `private` (the entry
/// carries the remote credential) and otherwise gets the process default.
fn write_text(path: &Path, text: &str, private: bool) -> Result<(), String> {
  let parent = path
    .parent()
    .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
  fs::create_dir_all(parent).map_err(|e| format!("Failed to create config dir: {e}"))?;
  let file_name = path
    .file_name()
    .and_then(|name| name.to_str())
    .ok_or_else(|| format!("{} has no file name", path.display()))?;
  let tmp = parent.join(format!(".{file_name}.donut-tmp"));
  let existing = fs::metadata(path).ok();
  // Owner-only from the first byte whenever the content is secret or the
  // target's own bits are about to be copied over it: a temp file created
  // with the default mode and tightened afterwards is readable by everyone
  // in between.
  let written = if private || existing.is_some() {
    crate::app_dirs::write_owner_only(&tmp, text.as_bytes())
  } else {
    fs::write(&tmp, text)
  };
  written.map_err(|e| format!("Failed to write config: {e}"))?;
  if let Some(metadata) = existing {
    if let Err(e) = fs::set_permissions(&tmp, metadata.permissions()) {
      let _ = fs::remove_file(&tmp);
      return Err(format!("Failed to keep config permissions: {e}"));
    }
  }
  if let Err(e) = fs::rename(&tmp, path) {
    let _ = fs::remove_file(&tmp);
    return Err(format!("Failed to save config: {e}"));
  }
  Ok(())
}

/// fx expects its config to be private: the directory 700 and the file 600,
/// matching what its own installer does.
#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> Result<(), String> {
  use std::os::unix::fs::PermissionsExt;
  if let Some(dir) = path.parent() {
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
      .map_err(|e| format!("Failed to restrict config dir permissions: {e}"))?;
  }
  fs::set_permissions(path, fs::Permissions::from_mode(0o600))
    .map_err(|e| format!("Failed to restrict config permissions: {e}"))
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) -> Result<(), String> {
  Ok(())
}

fn install_in(env: &AgentEnv, agent_id: &str, target: &McpTarget) -> Result<(), String> {
  let spec = spec_for(agent_id).ok_or_else(|| format!("Unknown agent: {agent_id}"))?;
  let path = config_path(env, agent_id)
    .ok_or_else(|| format!("Unable to resolve config path for {agent_id}"))?;
  let (mut document, was_empty) = read_document(&path, spec.format)?;
  document.set_entry(spec.config_key, &server_entry(agent_id, target))?;
  let mut text = document.to_text()?;
  if was_empty && !text.ends_with('\n') {
    text.push('\n');
  }
  write_text(&path, &text, target.bearer.is_some())?;
  if agent_id == "fx" {
    restrict_to_owner(&path)?;
  }
  Ok(())
}

fn uninstall_in(env: &AgentEnv, agent_id: &str) -> Result<(), String> {
  let spec = spec_for(agent_id).ok_or_else(|| format!("Unknown agent: {agent_id}"))?;
  let Some(path) = config_path(env, agent_id) else {
    return Ok(());
  };
  if !path.exists() {
    return Ok(());
  }
  let (mut document, _) = read_document(&path, spec.format)?;
  let names: Vec<String> = document
    .entries(spec.config_key)
    .into_iter()
    .filter(|(name, value)| is_donut_entry(name, value))
    .map(|(name, _)| name)
    .collect();
  if names.is_empty() {
    return Ok(());
  }
  document.remove_entries(spec.config_key, &names);
  write_text(&path, &document.to_text()?, false)
}

/// The endpoint a client's config points at. The entry named `donut-browser`
/// decides when it is ours; otherwise any entry with a Donut URL counts, so a
/// renamed entry still reads as connected (and `uninstall_in` removes it).
fn status_in(env: &AgentEnv, agent_id: &str) -> Option<McpEndpoint> {
  let spec = spec_for(agent_id)?;
  let path = config_path(env, agent_id)?;
  if !path.exists() {
    return None;
  }
  let (document, _) = read_document(&path, spec.format).ok()?;
  let entries = document.entries(spec.config_key);
  entries
    .iter()
    .find(|(name, _)| name == SERVER_NAME)
    .and_then(|(_, value)| endpoint_of_entry(value))
    .or_else(|| {
      entries
        .iter()
        .find_map(|(_, value)| endpoint_of_entry(value))
    })
}

fn process_env() -> Result<AgentEnv, String> {
  AgentEnv::from_process().ok_or_else(|| "Home directory unavailable".to_string())
}

pub fn install_generic(agent_id: &str, target: &McpTarget) -> Result<(), String> {
  install_in(&process_env()?, agent_id, target)
}

pub fn uninstall_generic(agent_id: &str) -> Result<(), String> {
  uninstall_in(&process_env()?, agent_id)
}

/// Ids of the file-based clients whose entry points at `endpoint`. Claude
/// Desktop is not included: its bundle is inspected by lib.rs.
pub fn agents_on_endpoint(endpoint: McpEndpoint) -> Vec<String> {
  let Some(env) = AgentEnv::from_process() else {
    return Vec::new();
  };
  AGENT_SPECS
    .iter()
    .filter(|spec| spec.id != "claude-desktop")
    .filter(|spec| status_in(&env, spec.id) == Some(endpoint))
    .map(|spec| spec.id.to_string())
    .collect()
}

pub fn list_agents_with_status(overrides: &[(&str, AgentStatus)]) -> Vec<McpAgentInfo> {
  let env = AgentEnv::from_process();
  AGENT_SPECS
    .iter()
    .map(|spec| {
      let status = overrides
        .iter()
        .find(|(id, _)| *id == spec.id)
        .map(|(_, status)| *status)
        .unwrap_or_else(|| {
          let endpoint = env.as_ref().and_then(|env| status_in(env, spec.id));
          AgentStatus {
            connected: endpoint.is_some(),
            endpoint,
          }
        });
      McpAgentInfo {
        id: spec.id.to_string(),
        display_name: spec.display_name.to_string(),
        category: spec.category,
        connected: status.connected,
        detected: env.as_ref().is_some_and(|env| detected(env, spec.id)),
        endpoint: status.endpoint,
        token_env: token_env_for(spec.id),
      }
    })
    .collect()
}

pub fn agent_exists(agent_id: &str) -> bool {
  spec_for(agent_id).is_some()
}

/// `<app support>/Claude`: where Claude Desktop keeps its config, its
/// extension bundles and the extension registry.
pub fn claude_desktop_dir() -> Option<PathBuf> {
  AgentEnv::from_process().map(|env| env.claude_desktop_dir())
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  const KEY: &str = "dmk_test_credential";
  const LOCAL_URL: &str = "http://127.0.0.1:51080/mcp/abc123";

  fn test_env(root: &Path, platform: Platform) -> AgentEnv {
    AgentEnv {
      platform,
      home: root.join("home"),
      appdata: None,
      xdg_config_home: None,
      codex_home: None,
      grok_home: None,
      kimi_code_home: None,
      cline_dir: None,
    }
  }

  fn remote() -> McpTarget {
    McpTarget::remote(KEY.to_string())
  }

  fn local() -> McpTarget {
    McpTarget::local(LOCAL_URL.to_string())
  }

  fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
  }

  fn entry_of(env: &AgentEnv, agent_id: &str) -> serde_json::Value {
    let spec = spec_for(agent_id).unwrap();
    let path = config_path(env, agent_id).unwrap();
    let (document, _) = read_document(&path, spec.format).unwrap();
    document
      .entries(spec.config_key)
      .into_iter()
      .find(|(name, _)| name == SERVER_NAME)
      .map(|(_, value)| value)
      .unwrap_or_else(|| panic!("{agent_id} has no {SERVER_NAME} entry"))
  }

  fn generic_ids() -> impl Iterator<Item = &'static str> {
    AGENT_SPECS
      .iter()
      .map(|spec| spec.id)
      .filter(|id| *id != "claude-desktop")
  }

  #[test]
  fn registry_has_the_twenty_clients() {
    let mut ids: Vec<&str> = AGENT_SPECS.iter().map(|spec| spec.id).collect();
    ids.sort_unstable();
    assert_eq!(
      ids,
      vec![
        "antigravity",
        "claude-code",
        "claude-desktop",
        "cline",
        "cline-cli",
        "codex",
        "cursor",
        "fx",
        "gemini-cli",
        "github-copilot-cli",
        "goose",
        "grok-build",
        "kilo-code",
        "kimi-code",
        "kiro-cli",
        "mcporter",
        "opencode",
        "vscode",
        "windsurf",
        "zed",
      ]
    );
  }

  #[test]
  fn paths_per_platform_match_the_registry() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let h = root.join("home");
    let cases: Vec<(Platform, &str, PathBuf)> = vec![
      (Platform::MacOs, "antigravity", h.join(".gemini/config/mcp_config.json")),
      (Platform::Linux, "antigravity", h.join(".gemini/config/mcp_config.json")),
      (Platform::Windows, "antigravity", h.join(".gemini/config/mcp_config.json")),
      (
        Platform::MacOs,
        "cline",
        h.join("Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"),
      ),
      (
        Platform::Linux,
        "cline",
        h.join(".config/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"),
      ),
      (
        Platform::Windows,
        "cline",
        h.join("AppData/Roaming/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"),
      ),
      (Platform::MacOs, "cline-cli", h.join(".cline/data/settings/cline_mcp_settings.json")),
      (Platform::Windows, "cline-cli", h.join(".cline/data/settings/cline_mcp_settings.json")),
      (Platform::MacOs, "claude-code", h.join(".claude.json")),
      (Platform::Linux, "claude-code", h.join(".claude.json")),
      (Platform::Windows, "claude-code", h.join(".claude.json")),
      (
        Platform::MacOs,
        "claude-desktop",
        h.join("Library/Application Support/Claude/claude_desktop_config.json"),
      ),
      (Platform::Linux, "claude-desktop", h.join(".config/Claude/claude_desktop_config.json")),
      (
        Platform::Windows,
        "claude-desktop",
        h.join("AppData/Roaming/Claude/claude_desktop_config.json"),
      ),
      (Platform::MacOs, "codex", h.join(".codex/config.toml")),
      (Platform::Windows, "codex", h.join(".codex/config.toml")),
      (Platform::MacOs, "cursor", h.join(".cursor/mcp.json")),
      (Platform::Windows, "cursor", h.join(".cursor/mcp.json")),
      (Platform::MacOs, "fx", h.join(".fx/mcp.json")),
      (Platform::Windows, "fx", h.join(".fx/mcp.json")),
      (Platform::MacOs, "gemini-cli", h.join(".gemini/settings.json")),
      (Platform::Windows, "gemini-cli", h.join(".gemini/settings.json")),
      (Platform::MacOs, "goose", h.join(".config/goose/config.yaml")),
      (Platform::Linux, "goose", h.join(".config/goose/config.yaml")),
      (
        Platform::Windows,
        "goose",
        h.join("AppData/Roaming/Block/goose/config/config.yaml"),
      ),
      (Platform::MacOs, "github-copilot-cli", h.join(".copilot/mcp-config.json")),
      (Platform::Windows, "github-copilot-cli", h.join(".copilot/mcp-config.json")),
      (Platform::MacOs, "grok-build", h.join(".grok/config.toml")),
      (Platform::Windows, "grok-build", h.join(".grok/config.toml")),
      (Platform::MacOs, "kilo-code", h.join(".config/kilo/kilo.json")),
      (Platform::Linux, "kilo-code", h.join(".config/kilo/kilo.json")),
      (Platform::Windows, "kilo-code", h.join(".config/kilo/kilo.json")),
      (Platform::MacOs, "kimi-code", h.join(".kimi-code/mcp.json")),
      (Platform::Windows, "kimi-code", h.join(".kimi-code/mcp.json")),
      (Platform::MacOs, "kiro-cli", h.join(".kiro/settings/mcp.json")),
      (Platform::Windows, "kiro-cli", h.join(".kiro/settings/mcp.json")),
      (Platform::MacOs, "mcporter", h.join(".mcporter/mcporter.json")),
      (Platform::Windows, "mcporter", h.join(".mcporter/mcporter.json")),
      (Platform::MacOs, "opencode", h.join(".config/opencode/opencode.jsonc")),
      (Platform::Linux, "opencode", h.join(".config/opencode/opencode.jsonc")),
      (Platform::Windows, "opencode", h.join(".config/opencode/opencode.jsonc")),
      (Platform::MacOs, "vscode", h.join("Library/Application Support/Code/User/mcp.json")),
      (Platform::Linux, "vscode", h.join(".config/Code/User/mcp.json")),
      (Platform::Windows, "vscode", h.join("AppData/Roaming/Code/User/mcp.json")),
      (Platform::MacOs, "windsurf", h.join(".codeium/windsurf/mcp_config.json")),
      (Platform::Windows, "windsurf", h.join(".codeium/windsurf/mcp_config.json")),
      (Platform::MacOs, "zed", h.join("Library/Application Support/Zed/settings.json")),
      (Platform::Linux, "zed", h.join(".config/zed/settings.json")),
      (Platform::Windows, "zed", h.join("AppData/Roaming/Zed/settings.json")),
    ];
    for (platform, agent_id, expected) in cases {
      let env = test_env(root, platform);
      assert_eq!(
        config_path(&env, agent_id).unwrap(),
        expected,
        "{agent_id} on {platform:?}"
      );
    }
  }

  #[test]
  fn env_overrides_move_the_config_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut env = test_env(root, Platform::Linux);
    env.xdg_config_home = Some(root.join("xdg"));
    env.codex_home = Some(root.join("codex-home"));
    env.grok_home = Some(root.join("grok-home"));
    env.kimi_code_home = Some(root.join("kimi-home"));
    env.cline_dir = Some(root.join("cline-dir"));

    let expect = |env: &AgentEnv, agent_id: &str, path: PathBuf| {
      assert_eq!(config_path(env, agent_id).unwrap(), path, "{agent_id}");
    };
    expect(&env, "codex", root.join("codex-home/config.toml"));
    expect(&env, "grok-build", root.join("grok-home/config.toml"));
    expect(&env, "kimi-code", root.join("kimi-home/mcp.json"));
    expect(
      &env,
      "cline-cli",
      root.join("cline-dir/data/settings/cline_mcp_settings.json"),
    );
    expect(&env, "vscode", root.join("xdg/Code/User/mcp.json"));
    expect(&env, "zed", root.join("xdg/zed/settings.json"));
    expect(&env, "goose", root.join("xdg/goose/config.yaml"));
    expect(&env, "kilo-code", root.join("xdg/kilo/kilo.json"));
    expect(&env, "github-copilot-cli", root.join("xdg/mcp-config.json"));
    expect(
      &env,
      "claude-desktop",
      root.join("xdg/Claude/claude_desktop_config.json"),
    );
    // OpenCode ignores XDG_CONFIG_HOME, and Goose on macOS does too.
    expect(
      &env,
      "opencode",
      root.join("home/.config/opencode/opencode.jsonc"),
    );
    env.platform = Platform::MacOs;
    expect(&env, "goose", root.join("home/.config/goose/config.yaml"));

    let mut windows = test_env(root, Platform::Windows);
    windows.appdata = Some(root.join("roaming"));
    assert_eq!(
      config_path(&windows, "vscode").unwrap(),
      root.join("roaming/Code/User/mcp.json")
    );
    assert_eq!(
      config_path(&windows, "goose").unwrap(),
      root.join("roaming/Block/goose/config/config.yaml")
    );
    assert_eq!(
      config_path(&windows, "zed").unwrap(),
      root.join("roaming/Zed/settings.json")
    );
    assert_eq!(
      config_path(&windows, "claude-desktop").unwrap(),
      root.join("roaming/Claude/claude_desktop_config.json")
    );
  }

  #[test]
  fn jsonc_variants_are_preferred_when_present() {
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::Linux);
    let home = &env.home;

    fs::create_dir_all(home.join(".config/opencode")).unwrap();
    fs::write(home.join(".config/opencode/opencode.json"), "{}").unwrap();
    assert_eq!(
      config_path(&env, "opencode").unwrap(),
      home.join(".config/opencode/opencode.json")
    );
    fs::write(home.join(".config/opencode/opencode.jsonc"), "{}").unwrap();
    assert_eq!(
      config_path(&env, "opencode").unwrap(),
      home.join(".config/opencode/opencode.jsonc")
    );

    fs::create_dir_all(home.join(".mcporter")).unwrap();
    fs::write(home.join(".mcporter/mcporter.jsonc"), "{}").unwrap();
    assert_eq!(
      config_path(&env, "mcporter").unwrap(),
      home.join(".mcporter/mcporter.jsonc")
    );
    fs::write(home.join(".mcporter/mcporter.json"), "{}").unwrap();
    assert_eq!(
      config_path(&env, "mcporter").unwrap(),
      home.join(".mcporter/mcporter.json")
    );

    fs::create_dir_all(home.join(".config/kilo")).unwrap();
    fs::write(home.join(".config/kilo/kilo.jsonc"), "{}").unwrap();
    assert_eq!(
      config_path(&env, "kilo-code").unwrap(),
      home.join(".config/kilo/kilo.jsonc")
    );
  }

  #[test]
  fn detection_follows_the_registry_rules() {
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::MacOs);
    let home = &env.home;
    for id in AGENT_SPECS.iter().map(|spec| spec.id) {
      assert!(!detected(&env, id), "{id} detected in an empty home");
    }

    // Gemini CLI alone must not make Antigravity look installed.
    fs::create_dir_all(home.join(".gemini")).unwrap();
    assert!(detected(&env, "gemini-cli"));
    assert!(!detected(&env, "antigravity"));
    fs::create_dir_all(home.join(".gemini/config")).unwrap();
    assert!(detected(&env, "antigravity"));

    fs::create_dir_all(home.join(".config/kilo")).unwrap();
    assert!(detected(&env, "kilo-code"));
    fs::create_dir_all(home.join(".codeium/windsurf")).unwrap();
    assert!(detected(&env, "windsurf"));
    fs::create_dir_all(home.join(".kiro")).unwrap();
    assert!(detected(&env, "kiro-cli"));
    fs::create_dir_all(home.join(".fx")).unwrap();
    assert!(detected(&env, "fx"));
    fs::create_dir_all(home.join(".grok")).unwrap();
    assert!(detected(&env, "grok-build"));
    fs::create_dir_all(home.join(".kimi-code")).unwrap();
    assert!(detected(&env, "kimi-code"));
    assert!(!detected(&env, "goose"));
    fs::create_dir_all(home.join(".config/goose")).unwrap();
    fs::write(home.join(".config/goose/config.yaml"), "").unwrap();
    assert!(detected(&env, "goose"));
    fs::create_dir_all(home.join("Library/Application Support/Claude")).unwrap();
    assert!(detected(&env, "claude-desktop"));
  }

  #[test]
  fn remote_shapes_put_the_credential_where_each_client_reads_it() {
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::Linux);
    let url = remote_mcp_url();
    let auth = json!({ "Authorization": format!("Bearer {KEY}") });
    let expected: Vec<(&str, serde_json::Value)> = vec![
      (
        "claude-code",
        json!({ "type": "http", "url": url, "headers": auth }),
      ),
      (
        "gemini-cli",
        json!({ "type": "http", "url": url, "headers": auth }),
      ),
      (
        "mcporter",
        json!({ "type": "http", "url": url, "headers": auth }),
      ),
      (
        "vscode",
        json!({ "type": "http", "url": url, "headers": auth }),
      ),
      ("cursor", json!({ "url": url, "headers": auth })),
      ("kiro-cli", json!({ "url": url, "headers": auth })),
      ("grok-build", json!({ "url": url, "headers": auth })),
      ("antigravity", json!({ "serverUrl": url, "headers": auth })),
      ("windsurf", json!({ "serverUrl": url, "headers": auth })),
      (
        "kimi-code",
        json!({ "transport": "http", "url": url, "headers": auth }),
      ),
      (
        "cline",
        json!({ "url": url, "type": "streamableHttp", "disabled": false, "headers": auth }),
      ),
      (
        "cline-cli",
        json!({ "url": url, "type": "streamableHttp", "disabled": false, "headers": auth }),
      ),
      (
        "github-copilot-cli",
        json!({ "type": "http", "url": url, "tools": ["*"], "headers": auth }),
      ),
      (
        "codex",
        json!({ "type": "http", "url": url, "http_headers": auth }),
      ),
      (
        "zed",
        json!({ "source": "custom", "type": "http", "url": url, "headers": auth }),
      ),
      (
        "opencode",
        json!({ "type": "remote", "url": url, "enabled": true, "headers": auth }),
      ),
      (
        "kilo-code",
        json!({ "type": "remote", "url": url, "enabled": true, "headers": auth }),
      ),
      (
        "goose",
        json!({
          "name": SERVER_NAME, "description": "", "type": "streamable_http", "uri": url,
          "headers": auth, "enabled": true, "timeout": 300
        }),
      ),
      (
        "fx",
        json!({ "type": "http", "url": url, "enabled": true, "bearer_token_env": FX_TOKEN_ENV }),
      ),
    ];
    assert_eq!(expected.len(), generic_ids().count());
    for (agent_id, shape) in expected {
      install_in(&env, agent_id, &remote()).unwrap();
      assert_eq!(entry_of(&env, agent_id), shape, "{agent_id}");
      assert_eq!(
        status_in(&env, agent_id),
        Some(McpEndpoint::Remote),
        "{agent_id}"
      );
    }
    let fx_text = read(&config_path(&env, "fx").unwrap());
    assert!(
      !fx_text.contains("Authorization"),
      "fx must never carry a literal header"
    );
  }

  #[test]
  fn jsonc_comments_indentation_and_trailing_commas_survive() {
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::MacOs);
    let path = config_path(&env, "zed").unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let original = "// Zed settings\n{\n    \"theme\": \"One Dark\",\n    /* keep me */\n    \"context_servers\": {\n        \"other\": { \"command\": \"x\", },\n    },\n    \"vim_mode\": true,\n}\n";
    fs::write(&path, original).unwrap();

    install_in(&env, "zed", &remote()).unwrap();
    let text = read(&path);
    assert!(text.starts_with("// Zed settings\n"));
    assert!(text.contains("/* keep me */"));
    assert!(text.contains("\n    \"vim_mode\": true,\n"));
    assert!(text.contains("\"other\": { \"command\": \"x\", }"));
    assert!(
      text.contains("\n        \"donut-browser\": {\n            \"source\": \"custom\","),
      "entry must use the file's four-space indent: {text}"
    );
    assert!(text.ends_with("}\n"));
    assert_eq!(status_in(&env, "zed"), Some(McpEndpoint::Remote));

    uninstall_in(&env, "zed").unwrap();
    let text = read(&path);
    assert!(text.starts_with("// Zed settings\n"));
    assert!(text.contains("/* keep me */"));
    assert!(!text.contains("donut-browser"));
    assert!(text.contains("\"other\": { \"command\": \"x\", }"));
    assert_eq!(status_in(&env, "zed"), None);
  }

  #[test]
  fn large_json_keeps_unrelated_keys_in_their_order() {
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::Linux);
    let path = config_path(&env, "claude-code").unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let original = "{\n  \"zeta\": 1,\n  \"projects\": {\"/a\": {\"allowedTools\": []}},\n  \"alpha\": \"b\"\n}\n";
    fs::write(&path, original).unwrap();
    install_in(&env, "claude-code", &remote()).unwrap();
    let text = read(&path);
    let zeta = text.find("\"zeta\"").unwrap();
    let projects = text.find("\"projects\"").unwrap();
    let alpha = text.find("\"alpha\"").unwrap();
    let servers = text.find("\"mcpServers\"").unwrap();
    assert!(
      zeta < projects && projects < alpha && alpha < servers,
      "{text}"
    );
    assert!(text.contains("{\"/a\": {\"allowedTools\": []}}"));
  }

  #[test]
  fn toml_comments_and_table_order_survive() {
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::Linux);
    let path = config_path(&env, "codex").unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let original = "# codex config\nmodel = \"o3\"\n\n[projects.\"/tmp/x\"]\ntrust_level = \"trusted\"\n\n[notice]\nseen = true\n\n[mcp_servers.other]\ncommand = \"npx\"\n";
    fs::write(&path, original).unwrap();

    install_in(&env, "codex", &remote()).unwrap();
    let text = read(&path);
    assert!(text.starts_with("# codex config\nmodel = \"o3\"\n"));
    let projects = text.find("[projects.\"/tmp/x\"]").unwrap();
    let notice = text.find("[notice]").unwrap();
    let other = text.find("[mcp_servers.other]").unwrap();
    let donut = text.find("[mcp_servers.donut-browser]").unwrap();
    assert!(
      projects < notice && notice < other && other < donut,
      "{text}"
    );
    assert!(
      !text.contains("\n[mcp_servers]\n"),
      "no bare header: {text}"
    );
    assert!(text.contains(&format!(
      "http_headers = {{ Authorization = \"Bearer {KEY}\" }}"
    )));
    assert_eq!(
      entry_of(&env, "codex")["http_headers"]["Authorization"],
      json!(format!("Bearer {KEY}"))
    );

    uninstall_in(&env, "codex").unwrap();
    let text = read(&path);
    assert!(text.starts_with("# codex config\n"));
    assert!(text.contains("[mcp_servers.other]\ncommand = \"npx\"\n"));
    assert!(!text.contains("donut-browser"));
  }

  #[test]
  fn grok_toml_entry_has_no_type_and_a_headers_table() {
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::Linux);
    install_in(&env, "grok-build", &remote()).unwrap();
    let text = read(&config_path(&env, "grok-build").unwrap());
    assert!(text.contains("[mcp_servers.donut-browser]\n"));
    assert!(text.contains(&format!("url = \"{}\"", remote_mcp_url())));
    assert!(text.contains(&format!("headers = {{ Authorization = \"Bearer {KEY}\" }}")));
    assert!(!text.contains("type ="));
  }

  #[test]
  fn goose_yaml_keeps_other_extensions_and_their_order() {
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::Linux);
    let path = config_path(&env, "goose").unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
      &path,
      "GOOSE_PROVIDER: openai\nextensions:\n  zeta:\n    enabled: true\n    type: builtin\n  alpha:\n    enabled: false\n    type: builtin\n",
    )
    .unwrap();
    install_in(&env, "goose", &remote()).unwrap();
    let text = read(&path);
    let provider = text.find("GOOSE_PROVIDER: openai").unwrap();
    let zeta = text.find("zeta:").unwrap();
    let alpha = text.find("alpha:").unwrap();
    let donut = text.find("donut-browser:").unwrap();
    assert!(provider < zeta && zeta < alpha && alpha < donut, "{text}");
    let name = text.find("name: donut-browser").unwrap();
    let uri = text.find("uri:").unwrap();
    let timeout = text.find("timeout: 300").unwrap();
    assert!(name < uri && uri < timeout, "{text}");
    assert_eq!(status_in(&env, "goose"), Some(McpEndpoint::Remote));

    uninstall_in(&env, "goose").unwrap();
    let text = read(&path);
    assert!(text.contains("zeta:") && text.contains("alpha:"));
    assert!(!text.contains("donut-browser"));
  }

  #[test]
  fn parse_failures_abort_and_leave_the_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::Linux);
    let cases = [
      ("cursor", "{ \"mcpServers\": { \"a\": }"),
      ("codex", "[mcp_servers\nbroken = "),
      ("goose", "extensions:\n  - [unclosed\n"),
    ];
    for (agent_id, broken) in cases {
      let path = config_path(&env, agent_id).unwrap();
      fs::create_dir_all(path.parent().unwrap()).unwrap();
      fs::write(&path, broken).unwrap();
      let error = install_in(&env, agent_id, &remote()).unwrap_err();
      assert!(error.contains("left untouched"), "{agent_id}: {error}");
      assert_eq!(read(&path), broken, "{agent_id} was rewritten");
      let error = uninstall_in(&env, agent_id).unwrap_err();
      assert!(error.contains("left untouched"), "{agent_id}: {error}");
      assert_eq!(read(&path), broken, "{agent_id} was rewritten on remove");
      assert_eq!(status_in(&env, agent_id), None);
    }
  }

  #[test]
  fn non_object_containers_are_an_error_not_an_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::Linux);
    let path = config_path(&env, "cursor").unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    for content in ["[1, 2]", "{ \"mcpServers\": \"nope\" }"] {
      fs::write(&path, content).unwrap();
      assert!(install_in(&env, "cursor", &remote()).is_err(), "{content}");
      assert_eq!(read(&path), content);
    }
  }

  #[test]
  fn empty_and_missing_files_become_a_fresh_object() {
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::Linux);
    let path = config_path(&env, "cursor").unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "").unwrap();
    install_in(&env, "cursor", &local()).unwrap();
    let text = read(&path);
    assert!(
      text.starts_with("{\n  \"mcpServers\": {\n    \"donut-browser\": {"),
      "{text}"
    );
    assert!(text.ends_with("}\n"));

    assert!(!config_path(&env, "vscode").unwrap().exists());
    install_in(&env, "vscode", &remote()).unwrap();
    assert_eq!(status_in(&env, "vscode"), Some(McpEndpoint::Remote));
    let text = read(&config_path(&env, "vscode").unwrap());
    assert!(text.ends_with("\n"));
    assert!(fs::read_to_string(config_path(&env, "codex").unwrap()).is_err());
    install_in(&env, "codex", &remote()).unwrap();
    assert_eq!(status_in(&env, "codex"), Some(McpEndpoint::Remote));
    install_in(&env, "goose", &remote()).unwrap();
    assert_eq!(status_in(&env, "goose"), Some(McpEndpoint::Remote));
  }

  #[test]
  fn reinstall_replaces_the_entry_wholesale() {
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::Linux);
    let path = config_path(&env, "cursor").unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
      &path,
      "{\"mcpServers\": {\"donut-browser\": {\"url\": \"http://127.0.0.1:1/mcp/old\", \"disabled\": true, \"env\": {\"X\": \"1\"}}}}",
    )
    .unwrap();
    assert_eq!(status_in(&env, "cursor"), Some(McpEndpoint::Local));
    install_in(&env, "cursor", &remote()).unwrap();
    let entry = entry_of(&env, "cursor");
    assert!(entry.get("disabled").is_none());
    assert!(entry.get("env").is_none());
    assert_eq!(entry["url"], json!(remote_mcp_url()));
    assert_eq!(status_in(&env, "cursor"), Some(McpEndpoint::Remote));

    install_in(&env, "cursor", &local()).unwrap();
    let entry = entry_of(&env, "cursor");
    assert!(entry.get("headers").is_none());
    assert_eq!(status_in(&env, "cursor"), Some(McpEndpoint::Local));
  }

  #[test]
  fn renamed_entries_are_detected_and_removed_while_foreign_ones_stay() {
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::Linux);
    let path = config_path(&env, "cursor").unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
      &path,
      format!(
        "{{\"mcpServers\": {{\"github\": {{\"url\": \"https://api.githubcopilot.com/mcp/\"}}, \"donut\": {{\"url\": \"{}\"}}, \"donut-browser\": {{\"url\": \"https://example.com/mcp\"}}}}}}",
        remote_mcp_url()
      ),
    )
    .unwrap();
    assert_eq!(status_in(&env, "cursor"), Some(McpEndpoint::Remote));
    uninstall_in(&env, "cursor").unwrap();
    assert_eq!(status_in(&env, "cursor"), None);
    let text = read(&path);
    assert!(text.contains("\"github\""));
    assert!(!text.contains("\"donut\""));
    assert!(!text.contains("\"donut-browser\""));

    // Nothing of ours left: removal is a no-op that does not rewrite the file.
    let before = read(&path);
    uninstall_in(&env, "cursor").unwrap();
    assert_eq!(read(&path), before);
    uninstall_in(&env, "vscode").unwrap();
    assert!(!config_path(&env, "vscode").unwrap().exists());
  }

  #[test]
  fn endpoint_of_url_recognises_both_endpoints_only() {
    assert_eq!(
      endpoint_of_url(&remote_mcp_url()),
      Some(McpEndpoint::Remote)
    );
    assert_eq!(
      endpoint_of_url(&format!("{}/", remote_mcp_url())),
      Some(McpEndpoint::Remote)
    );
    assert_eq!(
      endpoint_of_url("http://127.0.0.1:51080/mcp/tok"),
      Some(McpEndpoint::Local)
    );
    assert_eq!(
      endpoint_of_url("http://localhost:51080/mcp/tok"),
      Some(McpEndpoint::Local)
    );
    assert_eq!(
      endpoint_of_url("http://localhost:51080/mcp"),
      Some(McpEndpoint::Local)
    );
    assert_eq!(endpoint_of_url("http://127.0.0.1:51080/mcpx"), None);
    assert_eq!(endpoint_of_url("http://127.0.0.1:51080/api"), None);
    assert_eq!(endpoint_of_url("https://api.githubcopilot.com/mcp/"), None);
    assert_eq!(endpoint_of_url("http://evil.example/mcp/tok"), None);
    assert_eq!(
      endpoint_of_url("https://api.donutbrowser.com/api/mcp-bridge"),
      None
    );
    assert_eq!(McpEndpoint::parse("remote"), Some(McpEndpoint::Remote));
    assert_eq!(McpEndpoint::parse("local"), Some(McpEndpoint::Local));
    assert_eq!(McpEndpoint::parse("cloud"), None);
  }

  #[test]
  fn agent_info_reports_the_fx_token_variable() {
    assert_eq!(token_env_for("fx").as_deref(), Some(FX_TOKEN_ENV));
    assert_eq!(token_env_for("cursor"), None);
  }

  #[cfg(unix)]
  #[test]
  fn fx_config_is_private_to_the_owner() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::Linux);
    install_in(&env, "fx", &remote()).unwrap();
    let path = config_path(&env, "fx").unwrap();
    assert_eq!(
      fs::metadata(&path).unwrap().permissions().mode() & 0o777,
      0o600
    );
    assert_eq!(
      fs::metadata(path.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777,
      0o700
    );
  }

  #[cfg(unix)]
  #[test]
  fn a_first_time_config_carrying_the_credential_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::Linux);
    let path = config_path(&env, "cursor").unwrap();
    assert!(!path.exists());
    install_in(&env, "cursor", &remote()).unwrap();
    // No file existed to copy bits from, and the entry carries the remote
    // credential: the file is owner-only from its first byte, not after a
    // chmod that follows a world-readable write.
    assert_eq!(
      fs::metadata(&path).unwrap().permissions().mode() & 0o777,
      0o600
    );
  }

  #[cfg(unix)]
  #[test]
  fn a_first_time_local_config_keeps_the_default_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::Linux);
    // Whatever this process's umask makes of an ordinary new file: the local
    // entry carries no secret, so it is not tightened beyond that.
    let probe = dir.path().join("probe");
    fs::write(&probe, "").unwrap();
    let default_mode = fs::metadata(&probe).unwrap().permissions().mode() & 0o777;
    install_in(&env, "cursor", &local()).unwrap();
    let path = config_path(&env, "cursor").unwrap();
    assert_eq!(
      fs::metadata(&path).unwrap().permissions().mode() & 0o777,
      default_mode
    );
  }

  #[cfg(unix)]
  #[test]
  fn rewrites_keep_the_existing_permission_bits() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let env = test_env(dir.path(), Platform::Linux);
    let path = config_path(&env, "cursor").unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "{}").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    install_in(&env, "cursor", &remote()).unwrap();
    assert_eq!(
      fs::metadata(&path).unwrap().permissions().mode() & 0o777,
      0o640
    );
    assert!(!path.parent().unwrap().join(".mcp.json.donut-tmp").exists());
  }
}
