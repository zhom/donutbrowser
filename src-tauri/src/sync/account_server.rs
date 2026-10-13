//! A donut-sync server the customer runs, used while signed in to a paid
//! Donut account.
//!
//! The app signs in to that server with a short token the Donut API issues for
//! its one address, and the server asks the API whose the token is. The plan's
//! profile limit and the team's namespace, roles and group access thus apply
//! there as on Donut cloud, and every person who syncs needs their own seat. A
//! shared `SYNC_TOKEN` opens the whole bucket to anyone who has it, so it stays
//! the signed-out way to use a server.

use crate::settings_manager::SettingsManager;

/// The address in the one form the Donut API and donut-sync also write: scheme,
/// host and port as the URL parser writes them, then the path without trailing
/// slashes. A token is bound to this exact string.
pub fn normalize_server_url(raw: &str) -> Option<String> {
  let raw = raw.trim();
  if raw.is_empty() || raw.len() > 2048 {
    return None;
  }
  let url = url::Url::parse(raw).ok()?;
  if !matches!(url.scheme(), "http" | "https") {
    return None;
  }
  let host = url.host_str().filter(|host| !host.is_empty())?;
  if !url.username().is_empty() || url.password().is_some() {
    return None;
  }
  if url.query().is_some_and(|q| !q.is_empty()) || url.fragment().is_some_and(|f| !f.is_empty()) {
    return None;
  }
  let port = url
    .port()
    .map(|port| format!(":{port}"))
    .unwrap_or_default();
  Some(format!(
    "{}://{host}{port}{}",
    url.scheme(),
    url.path().trim_end_matches('/')
  ))
}

/// The own server set on this device, signed in or not.
fn stored() -> Option<String> {
  SettingsManager::instance()
    .load_settings()
    .ok()?
    .account_sync_server_url
}

/// The own server this signed-in account syncs with, or `None` for Donut
/// cloud.
pub async fn configured() -> Option<String> {
  if !crate::cloud_auth::CLOUD_AUTH.is_logged_in().await {
    return None;
  }
  stored()
}

/// The refusal for work that runs a profile on Donut's hosts. They take the
/// profile from Donut cloud and cannot reach the customer's server.
pub fn cloud_run_refusal() -> Option<String> {
  stored().map(|_| crate::backend_error("REMOTE_NEEDS_DONUT_SYNC"))
}

pub(crate) fn clear() {
  let manager = SettingsManager::instance();
  if let Ok(mut settings) = manager.load_settings() {
    if settings.account_sync_server_url.take().is_some() {
      let _ = manager.save_settings(&settings);
    }
  }
}

/// Check that `raw` is a server this account can sync with, and return its
/// address.
async fn check(raw: &str) -> Result<String, String> {
  let auth = &crate::cloud_auth::CLOUD_AUTH;
  if !auth.is_logged_in().await {
    return Err(crate::backend_error("ACCOUNT_SYNC_SIGN_IN_REQUIRED"));
  }
  if !auth.can_use_cloud_backup().await {
    return Err(crate::backend_error("ACCOUNT_SYNC_PLAN_REQUIRED"));
  }
  let server =
    normalize_server_url(raw).ok_or_else(|| crate::backend_error("SYNC_SERVER_URL_INVALID"))?;

  let probe = super::preflight::check_sync_server(&server).await;
  if !probe.server_reachable || probe.storage_ready == Some(false) {
    return Err(crate::backend_error("SYNC_SERVER_UNREACHABLE"));
  }
  if probe.storage_reachable == Some(false) {
    return Err(
      serde_json::json!({
        "code": "SYNC_SERVER_STORAGE_UNREACHABLE",
        "params": { "endpoint": probe.storage_endpoint.unwrap_or_default() },
      })
      .to_string(),
    );
  }
  match probe.account_url.as_deref().and_then(normalize_server_url) {
    None => return Err(crate::backend_error("SYNC_SERVER_NO_ACCOUNTS")),
    Some(address) if address != server => {
      return Err(
        serde_json::json!({
          "code": "SYNC_SERVER_ADDRESS_MISMATCH",
          "params": { "address": address },
        })
        .to_string(),
      );
    }
    Some(_) => {}
  }

  // The API decides who may sync. Asking now names a refusal here, not at the
  // first sync.
  auth.server_sync_token(&server).await?;
  Ok(server)
}

#[tauri::command]
pub async fn get_account_sync_server() -> Result<Option<String>, String> {
  Ok(stored())
}

/// Sync this signed-in account with its own server, or with Donut cloud again
/// when `server_url` is empty.
#[tauri::command]
pub async fn set_account_sync_server(server_url: Option<String>) -> Result<Option<String>, String> {
  let server = match server_url.as_deref().map(str::trim) {
    Some(raw) if !raw.is_empty() => Some(check(raw).await?),
    _ => None,
  };
  let manager = SettingsManager::instance();
  let mut settings = manager
    .load_settings()
    .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?;
  settings.account_sync_server_url = server.clone();
  manager
    .save_settings(&settings)
    .map_err(|e| crate::backend_error_with_detail("INTERNAL_ERROR", e))?;
  log::info!("Account sync server set own_server={}", server.is_some());
  Ok(server)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn writes_one_form_for_every_way_to_type_the_same_address() {
    // The same cases as the API and donut-sync: a token only matches when all
    // three write the address alike.
    for typed in [
      "https://Sync.Example.com",
      "https://sync.example.com/",
      " https://sync.example.com:443// ",
      "https://sync.example.com/?",
    ] {
      assert_eq!(
        normalize_server_url(typed).as_deref(),
        Some("https://sync.example.com"),
        "{typed}"
      );
    }
    assert_eq!(
      normalize_server_url("http://192.168.1.5:3929/donut/").as_deref(),
      Some("http://192.168.1.5:3929/donut")
    );
    assert_eq!(
      normalize_server_url("http://[::1]:3929").as_deref(),
      Some("http://[::1]:3929")
    );
  }

  #[test]
  fn refuses_what_cannot_be_a_server_address() {
    let long = format!("https://{}.com", "a".repeat(2100));
    for bad in [
      "",
      "sync.example.com",
      "ftp://sync.example.com",
      "https://user:pass@sync.example.com",
      "https://sync.example.com/?a=1",
      "https://sync.example.com/#x",
      long.as_str(),
    ] {
      assert_eq!(normalize_server_url(bad), None, "{bad}");
    }
  }
}
