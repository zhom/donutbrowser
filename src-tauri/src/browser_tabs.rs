//! The tabs of a browser running on this computer, for an agent or an API
//! client that takes it over: list them, open one, bring one to the front,
//! close one. Every Wayfern launch listens for DevTools on 127.0.0.1, so a
//! profile the person opened can be driven without a restart.
//!
//! The tab a caller picked is the one the MCP browser tools act on from then
//! on, until it closes or another is picked.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::profile::{BrowserProfile, ProfileManager};

const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

/// The tab each profile's tools act on, by profile id.
static SELECTED: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize, PartialEq, utoipa::ToSchema)]
pub struct BrowserTab {
  pub id: String,
  pub title: String,
  pub url: String,
  /// The tab the agent tools act on.
  pub selected: bool,
}

fn code(code: &str) -> String {
  crate::backend_error(code)
}

/// The DevTools port of the browser running for `profile` here, if any.
pub async fn local_port(profile: &BrowserProfile) -> Option<u16> {
  if !crate::browser_runner::browser_is_running_for(&profile.id.to_string()) {
    return None;
  }
  let path = profile.get_profile_data_path(&ProfileManager::instance().get_profiles_dir());
  crate::wayfern_manager::WayfernManager::instance()
    .get_cdp_port(&path.to_string_lossy())
    .await
}

fn http() -> reqwest::Client {
  reqwest::Client::builder()
    .timeout(HTTP_TIMEOUT)
    .no_proxy()
    .build()
    .unwrap_or_default()
}

fn unreachable(e: impl std::fmt::Display) -> String {
  crate::backend_error_with_detail("BROWSER_TABS_UNAVAILABLE", e)
}

/// Pages a person would call tabs: not DevTools, not extension backgrounds.
fn tabs_of(targets: &[Value], selected: Option<&str>) -> Vec<BrowserTab> {
  targets
    .iter()
    .filter(|t| t.get("type").and_then(Value::as_str) == Some("page"))
    .filter_map(|t| {
      let url = t.get("url").and_then(Value::as_str).unwrap_or_default();
      if url.starts_with("devtools://") {
        return None;
      }
      let id = t.get("id").and_then(Value::as_str)?.to_string();
      Some(BrowserTab {
        selected: selected == Some(id.as_str()),
        title: t
          .get("title")
          .and_then(Value::as_str)
          .unwrap_or_default()
          .to_string(),
        url: url.to_string(),
        id,
      })
    })
    .collect()
}

async fn targets(port: u16) -> Result<Vec<Value>, String> {
  http()
    .get(format!("http://127.0.0.1:{port}/json/list"))
    .send()
    .await
    .map_err(unreachable)?
    .json::<Vec<Value>>()
    .await
    .map_err(unreachable)
}

fn selected_for(profile_id: &str) -> Option<String> {
  SELECTED.lock().ok()?.as_ref()?.get(profile_id).cloned()
}

fn select(profile_id: &str, tab_id: Option<&str>) {
  if let Ok(mut guard) = SELECTED.lock() {
    let map = guard.get_or_insert_with(HashMap::new);
    match tab_id {
      Some(id) => map.insert(profile_id.to_string(), id.to_string()),
      None => map.remove(profile_id),
    };
  }
}

/// The page socket of the tab picked for `profile_id`, when it is still open.
pub fn selected_socket(profile_id: &str, targets: &[Value]) -> Option<String> {
  let selected = selected_for(profile_id)?;
  targets
    .iter()
    .find(|t| t.get("id").and_then(Value::as_str) == Some(selected.as_str()))
    .and_then(|t| t.get("webSocketDebuggerUrl").and_then(Value::as_str))
    .map(str::to_string)
}

async fn port_of(profile: &BrowserProfile) -> Result<u16, String> {
  local_port(profile)
    .await
    .ok_or_else(|| code("BROWSER_NOT_RUNNING"))
}

pub async fn list(profile: &BrowserProfile) -> Result<Vec<BrowserTab>, String> {
  let port = port_of(profile).await?;
  let profile_id = profile.id.to_string();
  let tabs = tabs_of(&targets(port).await?, selected_for(&profile_id).as_deref());
  if !tabs.iter().any(|tab| tab.selected) {
    select(&profile_id, None);
  }
  Ok(tabs)
}

/// Open `url` in a new tab, pick it, and bring it to the front.
pub async fn open(profile: &BrowserProfile, url: &str) -> Result<BrowserTab, String> {
  let port = port_of(profile).await?;
  let created: Value = http()
    .put(format!(
      "http://127.0.0.1:{port}/json/new?{}",
      urlencoding::encode(url)
    ))
    .send()
    .await
    .map_err(unreachable)?
    .json()
    .await
    .map_err(unreachable)?;
  let id = created
    .get("id")
    .and_then(Value::as_str)
    .ok_or_else(|| code("BROWSER_TABS_UNAVAILABLE"))?
    .to_string();
  select(&profile.id.to_string(), Some(&id));
  log::info!("Tab opened profile={}", profile.id);
  Ok(BrowserTab {
    id,
    title: String::new(),
    url: url.to_string(),
    selected: true,
  })
}

/// Pick the tab `tab_id` and bring it to the front.
pub async fn switch(profile: &BrowserProfile, tab_id: &str) -> Result<BrowserTab, String> {
  let port = port_of(profile).await?;
  let tab = tabs_of(&targets(port).await?, None)
    .into_iter()
    .find(|tab| tab.id == tab_id)
    .ok_or_else(|| code("BROWSER_TAB_NOT_FOUND"))?;
  let response = http()
    .get(format!(
      "http://127.0.0.1:{port}/json/activate/{}",
      urlencoding::encode(tab_id)
    ))
    .send()
    .await
    .map_err(unreachable)?;
  if !response.status().is_success() {
    return Err(code("BROWSER_TAB_NOT_FOUND"));
  }
  select(&profile.id.to_string(), Some(tab_id));
  Ok(BrowserTab {
    selected: true,
    ..tab
  })
}

pub async fn close(profile: &BrowserProfile, tab_id: &str) -> Result<(), String> {
  let port = port_of(profile).await?;
  let response = http()
    .get(format!(
      "http://127.0.0.1:{port}/json/close/{}",
      urlencoding::encode(tab_id)
    ))
    .send()
    .await
    .map_err(unreachable)?;
  if !response.status().is_success() {
    return Err(code("BROWSER_TAB_NOT_FOUND"));
  }
  if selected_for(&profile.id.to_string()).as_deref() == Some(tab_id) {
    select(&profile.id.to_string(), None);
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn listing() -> Vec<Value> {
    vec![
      json!({ "id": "A", "type": "page", "title": "Shop", "url": "https://shop.example/", "webSocketDebuggerUrl": "ws://a" }),
      json!({ "id": "D", "type": "page", "title": "DevTools", "url": "devtools://devtools/x" }),
      json!({ "id": "S", "type": "service_worker", "url": "https://shop.example/sw.js" }),
      json!({ "id": "B", "type": "page", "title": "Mail", "url": "https://mail.example/", "webSocketDebuggerUrl": "ws://b" }),
    ]
  }

  #[test]
  fn tabs_are_pages_a_person_sees() {
    let tabs = tabs_of(&listing(), Some("B"));
    assert_eq!(
      tabs.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
      ["A", "B"]
    );
    assert!(tabs[1].selected && !tabs[0].selected);
  }

  #[test]
  fn the_picked_tab_is_the_one_the_tools_drive() {
    select("p-tabs-test", Some("B"));
    assert_eq!(
      selected_socket("p-tabs-test", &listing()).as_deref(),
      Some("ws://b")
    );
    select("p-tabs-test", Some("gone"));
    assert_eq!(selected_socket("p-tabs-test", &listing()), None);
    select("p-tabs-test", None);
  }
}
