//! The move from the removed local MCP server to remote MCP.
//!
//! Nothing here rewrites a client. The desktop offers the move in a dialog,
//! at most once per cloud account, and the dialog runs it through the
//! existing commands (`start_mcp_remote_bridge`, the credential commands and
//! `add_mcp_to_agent`) so every client reports its own result.

use serde::Serialize;

use crate::cloud_auth::CLOUD_AUTH;
use crate::mcp_integrations::McpEndpoint;
use crate::settings_manager::{AppSettings, SettingsManager};

/// What the desktop needs to decide whether to offer the move.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct McpMigrationOffer {
  /// Local MCP is in use here, and the signed-in account may use remote MCP.
  pub eligible: bool,
  /// `eligible`, and this account has not had the offer yet.
  pub due: bool,
  /// The legacy `mcp_enabled` flag, which keeps the local tombstone bound.
  pub local_server_enabled: bool,
  /// Ids of the clients whose Donut entry still points at the local server.
  pub local_clients: Vec<String>,
}

fn local_mcp_in_use(settings: &AppSettings, local_clients: &[String]) -> bool {
  settings.mcp_enabled || !local_clients.is_empty()
}

/// The decision itself. `account_id` is the signed-in cloud user, and
/// `entitled` is the server's answer for that account.
fn offer_for(
  settings: &AppSettings,
  account_id: Option<&str>,
  local_clients: Vec<String>,
  entitled: bool,
) -> McpMigrationOffer {
  let eligible = account_id.is_some() && entitled && local_mcp_in_use(settings, &local_clients);
  let offered = account_id.is_some_and(|id| {
    settings
      .mcp_migration_offered_for
      .iter()
      .any(|seen| seen == id)
  });
  McpMigrationOffer {
    eligible,
    due: eligible && !offered,
    local_server_enabled: settings.mcp_enabled,
    local_clients,
  }
}

/// Remembers the offer as made to `account_id`. Answers whether the settings
/// changed, so a repeated call writes nothing.
fn record_offered(settings: &mut AppSettings, account_id: &str) -> bool {
  if settings
    .mcp_migration_offered_for
    .iter()
    .any(|id| id == account_id)
  {
    return false;
  }
  settings
    .mcp_migration_offered_for
    .push(account_id.to_string());
  true
}

/// The server's answer, or no. The move is never offered on a guess: the
/// cached entitlement is wrong for a team member, see
/// `fetch_remote_control_entitlement`.
async fn remote_mcp_entitled() -> bool {
  match CLOUD_AUTH.fetch_remote_control_entitlement().await {
    Ok(entitled) => entitled,
    Err(e) => {
      log::warn!("[mcp-migration] Could not ask whether remote MCP is available: {e}");
      false
    }
  }
}

fn load_settings() -> Result<AppSettings, String> {
  SettingsManager::instance()
    .load_settings()
    .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))
}

#[tauri::command]
pub async fn get_mcp_migration_offer() -> Result<McpMigrationOffer, String> {
  let settings = load_settings()?;
  let local_clients = crate::mcp_clients_on(McpEndpoint::Local);
  let account_id = CLOUD_AUTH.get_user().await.map(|state| state.user.id);
  // A network call, so it is made only when there is something to move and
  // an account to move it for.
  let entitled = account_id.is_some()
    && local_mcp_in_use(&settings, &local_clients)
    && remote_mcp_entitled().await;
  Ok(offer_for(
    &settings,
    account_id.as_deref(),
    local_clients,
    entitled,
  ))
}

#[tauri::command]
pub async fn mark_mcp_migration_offered() -> Result<(), String> {
  let Some(state) = CLOUD_AUTH.get_user().await else {
    return Err(crate::backend_error("MCP_REMOTE_REQUIRES_SIGN_IN"));
  };
  let mut settings = load_settings()?;
  if record_offered(&mut settings, &state.user.id) {
    SettingsManager::instance()
      .save_settings(&settings)
      .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?;
  }
  Ok(())
}

/// Unbind the local tombstone and clear the flag that binds it at launch.
///
/// Safe to call when nothing is bound: the tombstone may have failed to bind,
/// and the flag is what the next launch reads.
#[tauri::command]
pub async fn turn_off_local_mcp_server(app_handle: tauri::AppHandle) -> Result<(), String> {
  let server = crate::mcp_server::McpServer::instance();
  if server.is_running() {
    if let Err(e) = server.stop().await {
      if !e.contains("MCP_SERVER_NOT_RUNNING") {
        return Err(e);
      }
    }
  }
  let manager = SettingsManager::instance();
  let mut settings = load_settings()?;
  if settings.mcp_enabled {
    settings.mcp_enabled = false;
    manager
      .save_settings(&settings)
      .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?;
  }
  manager
    .remove_mcp_token(&app_handle)
    .await
    .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))
}

/// Tell anyone still on the removed local server that it is gone.
///
/// An account that can move to remote MCP is not told here: the move dialog
/// explains the change and does the move, and a notice under it would say the
/// same thing twice.
pub async fn announce_local_mcp_removal() {
  let Ok(settings) = SettingsManager::instance().load_settings() else {
    return;
  };
  if !local_mcp_in_use(&settings, &crate::mcp_clients_on(McpEndpoint::Local)) {
    return;
  }
  if CLOUD_AUTH.is_logged_in().await && remote_mcp_entitled().await {
    log::info!(
      "[mcp-migration] Local MCP is still in use; the move to remote MCP is offered in the app"
    );
    return;
  }
  let _ = crate::events::emit_empty(crate::mcp_server::LOCAL_MCP_DEPRECATED_EVENT);
}

#[cfg(test)]
mod tests {
  use super::*;

  fn local(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| id.to_string()).collect()
  }

  #[test]
  fn the_offer_needs_local_use_a_signed_in_account_and_remote_mcp() {
    let settings = AppSettings::default();
    let offer = offer_for(&settings, Some("u1"), local(&["cursor"]), true);
    assert!(offer.eligible && offer.due);
    assert_eq!(offer.local_clients, local(&["cursor"]));
    assert!(!offer.local_server_enabled);

    assert_eq!(
      offer_for(&settings, Some("u1"), local(&["cursor"]), false),
      McpMigrationOffer {
        eligible: false,
        due: false,
        local_server_enabled: false,
        local_clients: local(&["cursor"]),
      },
      "an account without remote MCP is never offered the move"
    );
    assert!(
      !offer_for(&settings, None, local(&["cursor"]), true).eligible,
      "nobody signed in, nobody to move"
    );
    assert!(
      !offer_for(&settings, Some("u1"), Vec::new(), true).eligible,
      "nothing on the local server, nothing to move"
    );
  }

  #[test]
  fn the_local_server_flag_alone_is_local_use() {
    let settings = AppSettings {
      mcp_enabled: true,
      ..AppSettings::default()
    };
    let offer = offer_for(&settings, Some("u1"), Vec::new(), true);
    assert!(offer.due);
    assert!(offer.local_server_enabled);
    assert!(offer.local_clients.is_empty());
  }

  #[test]
  fn the_offer_is_made_once_per_account() {
    let mut settings = AppSettings::default();
    assert!(record_offered(&mut settings, "u1"));
    assert!(
      !record_offered(&mut settings, "u1"),
      "a repeat writes nothing"
    );
    assert_eq!(settings.mcp_migration_offered_for, local(&["u1"]));

    let offered = offer_for(&settings, Some("u1"), local(&["cursor"]), true);
    assert!(
      offered.eligible && !offered.due,
      "still eligible, so the Integrations page can offer it by hand"
    );
    assert!(
      offer_for(&settings, Some("u2"), local(&["cursor"]), true).due,
      "another account on the same desktop has its own offer"
    );
  }

  #[test]
  fn settings_from_before_the_offer_parse_with_nobody_offered() {
    let settings: AppSettings =
      serde_json::from_str(r#"{ "mcp_enabled": true }"#).expect("old settings parse");
    assert!(settings.mcp_migration_offered_for.is_empty());
    assert!(offer_for(&settings, Some("u1"), Vec::new(), true).due);
  }
}
