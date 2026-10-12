//! Toolsets: named groups of MCP tools.
//!
//! A client that names toolsets when it connects (`?toolsets=network,cookies`
//! on the endpoint URL) is shown the core tools plus those groups, so it loads
//! far fewer tool schemas into its context. A client that names none is shown
//! every tool, as before.
//!
//! The selection only shapes `tools/list`. A call to a tool outside it still
//! works: hiding a tool saves the client tokens, it is not a permission.

use std::collections::BTreeSet;

pub const CORE: &str = "core";

/// Every desktop tool, each in exactly one toolset.
pub const TOOLSETS: &[(&str, &[&str])] = &[
  (
    CORE,
    &[
      // Profiles
      "list_profiles",
      "get_profile",
      "get_profile_status",
      "create_profile",
      "update_profile",
      "delete_profile",
      "bulk_update_profiles",
      "list_tags",
      "list_groups",
      "get_group",
      "list_profile_statuses",
      "list_profile_templates",
      "create_profiles_from_template",
      // Browser control
      "run_profile",
      "kill_profile",
      "batch_run_profiles",
      "batch_stop_profiles",
      "navigate",
      "go_back",
      "scroll",
      "press_key",
      "wait_for",
      "screenshot",
      "evaluate_javascript",
      "click_element",
      "type_text",
      "get_page_content",
      "get_page_info",
      "get_interactive_elements",
      "click_by_index",
      "type_by_index",
      "perceive_page",
      "resolve_locator",
      "click_locator",
      "type_locator",
      "extract_structured",
      "pick_element",
      // The person watching
      "ask_human",
      "request_human_help",
      "wait_for_human",
      "get_human_updates",
      "report_progress",
      "send_feedback",
    ],
  ),
  (
    "organize",
    &[
      "create_group",
      "update_group",
      "delete_group",
      "assign_profiles_to_group",
      "detect_browser_profiles",
      "import_browser_profiles",
    ],
  ),
  (
    "network",
    &[
      "list_proxies",
      "get_proxy",
      "create_proxy",
      "update_proxy",
      "delete_proxy",
      "export_proxies",
      "import_proxies",
      "rotate_proxy_ip",
      "distribute_proxies",
      "import_vpn",
      "list_vpn_configs",
      "delete_vpn",
      "connect_vpn",
      "disconnect_vpn",
      "get_vpn_status",
      "update_profile_proxy_bypass_rules",
      "update_profile_dns_blocklist",
      "get_dns_blocklist_status",
    ],
  ),
  (
    "fingerprint",
    &["get_profile_fingerprint", "update_profile_fingerprint"],
  ),
  (
    "extensions",
    &[
      "list_extensions",
      "list_extension_groups",
      "add_extension",
      "update_extension",
      "create_extension_group",
      "update_extension_group",
      "add_extension_to_group",
      "remove_extension_from_group",
      "delete_extension",
      "delete_extension_group",
      "assign_extension_group_to_profile",
    ],
  ),
  (
    "cookies",
    &[
      "import_profile_cookies",
      "list_cookie_bot_schedules",
      "get_cookie_bot_schedule",
      "set_cookie_bot_schedule",
      "delete_cookie_bot_schedule",
      "check_cookie_bot_conflicts",
      "list_cookie_bot_runs",
      "run_cookie_bot_now",
      "cancel_cookie_bot_run",
      "list_cookie_bot_presets",
      "get_cookie_bot_usage",
    ],
  ),
  (
    "sync",
    &[
      "get_team_locks",
      "get_team_lock_status",
      "start_sync_session",
      "stop_sync_session",
      "get_sync_sessions",
      "remove_sync_follower",
    ],
  ),
  (
    "remote",
    &[
      "run_profile_remote",
      "stop_remote_session",
      "list_remote_sessions",
      "get_remote_session",
      "get_remote_hours_quota",
    ],
  ),
];

/// Toolsets whose tools Donut cloud serves, not this desktop. Known here so
/// a selection that names them keeps them.
pub const CLOUD_TOOLSETS: &[&str] = &["recipes"];

/// The toolsets a person can add to core, in display order.
pub fn optional_toolsets() -> Vec<&'static str> {
  TOOLSETS
    .iter()
    .map(|(set, _)| *set)
    .filter(|set| *set != CORE)
    .chain(CLOUD_TOOLSETS.iter().copied())
    .collect()
}

/// A saved choice in display order, or `None` when a name is unknown. An
/// empty choice, or every optional toolset, is all tools; `core` alone is the
/// core tools only.
pub fn normalize_choice(names: &[String]) -> Option<Vec<String>> {
  let optional = optional_toolsets();
  if names
    .iter()
    .any(|name| name != CORE && !optional.contains(&name.as_str()))
  {
    return None;
  }
  let chosen: Vec<String> = optional
    .iter()
    .filter(|set| names.iter().any(|name| name == *set))
    .map(|set| set.to_string())
    .collect();
  Some(if chosen.len() == optional.len() || names.is_empty() {
    Vec::new()
  } else if chosen.is_empty() {
    vec![CORE.to_string()]
  } else {
    chosen
  })
}

/// The toolsets a client asked for, core always among them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection(BTreeSet<&'static str>);

/// Read `core,network` (any case, any spacing). Nothing, an empty value or
/// `all` selects every tool; an unknown name is ignored.
pub fn parse(raw: Option<&str>) -> Option<Selection> {
  let raw = raw?.trim();
  if raw.is_empty() {
    return None;
  }
  let mut chosen = BTreeSet::from([CORE]);
  for name in raw.split(',').map(|name| name.trim().to_ascii_lowercase()) {
    if name == "all" {
      return None;
    }
    let known = TOOLSETS
      .iter()
      .map(|(set, _)| *set)
      .chain(CLOUD_TOOLSETS.iter().copied())
      .find(|set| *set == name);
    if let Some(set) = known {
      chosen.insert(set);
    }
  }
  Some(Selection(chosen))
}

pub fn toolset_of(tool: &str) -> Option<&'static str> {
  TOOLSETS
    .iter()
    .find(|(_, tools)| tools.contains(&tool))
    .map(|(set, _)| *set)
}

impl Selection {
  pub fn includes_tool(&self, tool: &str) -> bool {
    toolset_of(tool).is_some_and(|set| self.0.contains(set))
  }

  pub fn names(&self) -> Vec<&'static str> {
    self.0.iter().copied().collect()
  }

  /// The toolsets left out, for the note that tells an agent how to get them.
  pub fn missing(&self) -> Vec<&'static str> {
    TOOLSETS
      .iter()
      .map(|(set, _)| *set)
      .chain(CLOUD_TOOLSETS.iter().copied())
      .filter(|set| !self.0.contains(set))
      .collect()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn no_selection_means_every_tool() {
    assert_eq!(parse(None), None);
    assert_eq!(parse(Some("  ")), None);
    assert_eq!(parse(Some("network,ALL")), None);
  }

  #[test]
  fn core_is_always_selected_and_unknown_names_are_ignored() {
    let selection = parse(Some(" Network , nonsense,recipes")).unwrap();
    assert_eq!(selection.names(), ["core", "network", "recipes"]);
    assert!(selection.includes_tool("list_profiles"));
    assert!(selection.includes_tool("rotate_proxy_ip"));
    assert!(!selection.includes_tool("list_extensions"));
    assert!(!selection.includes_tool("no_such_tool"));
    assert!(selection.missing().contains(&"extensions"));
    assert!(!selection.missing().contains(&"recipes"));
  }

  #[test]
  fn a_saved_choice_is_ordered_and_all_of_them_means_every_tool() {
    let owned = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    assert_eq!(
      normalize_choice(&owned(&["cookies", "network", "network"])),
      Some(owned(&["network", "cookies"]))
    );
    assert_eq!(normalize_choice(&owned(&["core"])), Some(owned(&["core"])));
    assert_eq!(
      normalize_choice(&owned(&["core", "sync"])),
      Some(owned(&["sync"]))
    );
    assert_eq!(normalize_choice(&[]), Some(Vec::new()));
    assert_eq!(normalize_choice(&owned(&["nope"])), None);
    let every: Vec<String> = optional_toolsets().iter().map(|s| s.to_string()).collect();
    assert_eq!(normalize_choice(&every), Some(Vec::new()));
  }

  #[test]
  fn the_integrations_page_offers_the_same_toolsets() {
    let source = include_str!("../../src/lib/mcp.ts");
    let list = source
      .split_once("export const OPTIONAL_TOOLSETS = [")
      .and_then(|(_, rest)| rest.split_once(']'))
      .map(|(list, _)| list)
      .expect("OPTIONAL_TOOLSETS in src/lib/mcp.ts");
    let offered: Vec<&str> = list
      .split(',')
      .map(|item| item.trim().trim_matches('"'))
      .filter(|item| !item.is_empty())
      .collect();
    assert_eq!(offered, optional_toolsets());
  }

  #[test]
  fn a_tool_belongs_to_one_toolset() {
    let mut seen = std::collections::HashSet::new();
    for (_, tools) in TOOLSETS {
      for tool in *tools {
        assert!(seen.insert(*tool), "{tool} is in two toolsets");
      }
    }
  }
}
