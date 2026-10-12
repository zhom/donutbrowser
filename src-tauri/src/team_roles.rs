//! What the signed-in team role may change on this computer.
//!
//! A `launcher` may open the team's profiles and keep what a session changes
//! (cookies, history, Local Storage), never change a profile's settings or the
//! team's proxies, groups, VPNs, extensions, templates or statuses. The sync
//! server refuses those writes whatever this side does; this mirror refuses the
//! edit up front, so the change does not appear here and then revert.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::cloud_auth::CloudUser;

static READ_ONLY: AtomicBool = AtomicBool::new(false);

/// The commands that change team data. Session commands (launch, kill, open a
/// URL, cookie and storage-state import, proxy IP rotation) stay open.
const TEAM_DATA_COMMANDS: &[&str] = &[
  // Profiles
  "create_browser_profile_new",
  "clone_profile",
  "rename_profile",
  "delete_profile",
  "delete_selected_profiles",
  "restore_trashed_profile",
  "purge_trashed_profile",
  "empty_trash",
  "update_profile_proxy",
  "update_profile_vpn",
  "update_profile_tags",
  "update_profile_note",
  "update_profile_clear_on_close",
  "update_profile_launch_hook",
  "update_profile_window_color",
  "update_profile_proxy_bypass_rules",
  "update_profile_dns_blocklist",
  "update_profile_dns_server",
  "update_profile_start_urls",
  "update_wayfern_config",
  "match_profile_fingerprint_to_exit",
  "bulk_update_profiles",
  "bulk_create_profiles",
  "import_browser_profiles",
  "import_profile_archive",
  "distribute_proxies_to_profiles",
  "assign_profiles_to_group",
  "assign_extension_group_to_profile",
  "apply_group_bookmarks_to_profile",
  "set_profile_sync_mode",
  "set_profiles_status",
  "add_profile_totp",
  "delete_profile_totp",
  "set_profile_password",
  "change_profile_password",
  "remove_profile_password",
  "create_restore_point",
  "delete_restore_point",
  "restore_restore_point",
  // Proxies
  "create_stored_proxy",
  "update_stored_proxy",
  "delete_stored_proxy",
  "import_proxies_json",
  "import_proxies_from_parsed",
  "set_proxy_rotation_url",
  "set_proxy_sync_enabled",
  "create_cloud_location_proxy",
  // Groups
  "create_profile_group",
  "update_profile_group",
  "delete_profile_group",
  "set_group_bookmarks",
  "set_group_sync_enabled",
  // VPNs
  "import_vpn_config",
  "create_vpn_config_manual",
  "update_vpn_config",
  "delete_vpn_config",
  "set_vpn_sync_enabled",
  // Extensions
  "add_extension",
  "add_unpacked_extension",
  "update_extension",
  "update_extension_from_path",
  "delete_extension",
  "create_extension_group",
  "update_extension_group",
  "delete_extension_group",
  "add_extension_to_group",
  "remove_extension_from_group",
  "set_extension_sync_enabled",
  "set_extension_group_sync_enabled",
  // Templates and statuses
  "create_profile_template",
  "update_profile_template",
  "delete_profile_template",
  "save_profile_template_from_profile",
  "create_profile_status",
  "update_profile_status",
  "delete_profile_status",
];

/// The MCP tools that change team data, refused the same way.
const TEAM_DATA_TOOLS: &[&str] = &[
  "create_profile",
  "import_browser_profiles",
  "update_profile",
  "delete_profile",
  "bulk_update_profiles",
  "create_profiles_from_template",
  "update_profile_fingerprint",
  "update_profile_proxy_bypass_rules",
  "update_profile_dns_blocklist",
  "assign_profiles_to_group",
  "assign_extension_group_to_profile",
  "distribute_proxies",
  "create_group",
  "update_group",
  "delete_group",
  "create_proxy",
  "update_proxy",
  "delete_proxy",
  "import_proxies",
  "import_vpn",
  "delete_vpn",
  "add_extension",
  "update_extension",
  "delete_extension",
  "create_extension_group",
  "update_extension_group",
  "delete_extension_group",
  "add_extension_to_group",
  "remove_extension_from_group",
];

/// Whether `user` holds a seat that may run the team's profiles but not change
/// them.
pub fn is_launcher(user: &CloudUser) -> bool {
  user.team_id.is_some() && user.team_role.as_deref() == Some("launcher")
}

/// Record the signed-in account, or `None` once signed out.
pub fn observe(user: Option<&CloudUser>) {
  let read_only = user.is_some_and(is_launcher);
  if READ_ONLY.swap(read_only, Ordering::Relaxed) != read_only {
    log::info!("Team role changed read_only={read_only}");
  }
}

/// Whether this computer may only read the team's data.
pub fn is_read_only() -> bool {
  // The suites cannot sign in to a real team. A marker file in the session's
  // data root stands in for the launcher seat while it exists.
  #[cfg(feature = "e2e")]
  if std::env::var_os("DONUTBROWSER_DATA_ROOT").is_some_and(|root| {
    std::path::Path::new(&root)
      .join("e2e-team-read-only")
      .exists()
  }) {
    return true;
  }
  std::sync::LazyLock::force(&crate::cloud_auth::CLOUD_AUTH);
  READ_ONLY.load(Ordering::Relaxed)
}

/// The refusal for `command`, when the signed-in role may not run it.
pub fn refusal(command: &str) -> Option<String> {
  if !TEAM_DATA_COMMANDS.contains(&command) || !is_read_only() {
    return None;
  }
  log::info!("Edit refused, team role is read-only command={command}");
  Some(crate::backend_error("TEAM_ROLE_READ_ONLY"))
}

/// `handler`, refusing the team-data commands while the role is read-only.
pub fn guarded<R: tauri::Runtime>(
  handler: impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
  move |invoke| match refusal(invoke.message.command()) {
    Some(refused) => {
      invoke.resolver.reject(refused);
      true
    }
    None => handler(invoke),
  }
}

/// The refusal for the MCP tool `tool`, when the signed-in role may not run it.
pub fn tool_refusal(tool: &str) -> Option<String> {
  if !TEAM_DATA_TOOLS.contains(&tool) || !is_read_only() {
    return None;
  }
  log::info!("MCP tool refused, team role is read-only tool={tool}");
  Some(crate::backend_error("TEAM_ROLE_READ_ONLY"))
}

#[cfg(test)]
pub(crate) fn team_data_commands() -> &'static [&'static str] {
  TEAM_DATA_COMMANDS
}

#[cfg(test)]
mod tests {
  use super::*;

  fn user(team_id: Option<&str>, role: Option<&str>) -> CloudUser {
    serde_json::from_value(serde_json::json!({
      "id": "u1",
      "email": "member@example.com",
      "plan": "free",
      "subscriptionStatus": "active",
      "profileLimit": 0,
      "cloudProfilesUsed": 0,
      "proxyBandwidthLimitMb": 0,
      "proxyBandwidthUsedMb": 0,
      "teamId": team_id,
      "teamRole": role,
    }))
    .unwrap()
  }

  #[test]
  fn only_a_team_launcher_is_read_only() {
    assert!(is_launcher(&user(Some("t1"), Some("launcher"))));
    assert!(!is_launcher(&user(Some("t1"), Some("member"))));
    assert!(!is_launcher(&user(Some("t1"), None)));
    // A stale role from a team the account has left gives no restriction.
    assert!(!is_launcher(&user(None, Some("launcher"))));
  }

  #[test]
  fn every_guarded_command_is_registered() {
    let lib = include_str!("lib.rs");
    let start = lib.find("tauri::generate_handler![").unwrap();
    let registry = &lib[start..start + lib[start..].find("])").unwrap()];
    let registered: Vec<&str> = registry
      .lines()
      .map(|line| line.trim().trim_end_matches(','))
      .map(|line| line.rsplit("::").next().unwrap_or(line))
      .collect();
    for command in team_data_commands() {
      assert!(registered.contains(command), "{command} is not registered");
    }
  }

  #[test]
  fn every_guarded_tool_exists() {
    let tools: Vec<String> = crate::mcp_server::McpServer::new()
      .get_tools()
      .into_iter()
      .map(|tool| tool.name)
      .collect();
    for tool in TEAM_DATA_TOOLS {
      assert!(
        tools.iter().any(|name| name == tool),
        "{tool} is not an MCP tool"
      );
    }
    for tool in [
      "run_profile",
      "kill_profile",
      "navigate",
      "import_profile_cookies",
    ] {
      assert!(!TEAM_DATA_TOOLS.contains(&tool), "{tool}");
    }
  }

  #[test]
  fn session_commands_stay_open() {
    for command in [
      "launch_browser_profile",
      "kill_browser_profile",
      "open_url_with_profile",
      "import_pasted_cookies",
      "import_profile_storage_state",
      "rotate_proxy_ip",
    ] {
      assert!(!team_data_commands().contains(&command), "{command}");
    }
  }
}
