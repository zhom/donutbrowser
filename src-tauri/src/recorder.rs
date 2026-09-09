//! Recording what a person does, as a recipe.
//!
//! The browser reports real input at the browser-process level
//! (`Wayfern.enableInputCapture` / `Wayfern.inputCaptured`), so a recording
//! sees what the user actually did rather than what a page chose to expose.
//! Each event is turned into the same typed step the agent recipes API
//! validates, so a recording can be saved as a recipe and replayed unchanged.
//!
//! Two rules shape everything here:
//!
//! * **A click becomes a locator, not a coordinate.** Coordinates are useless
//!   on the next window size; the element under the pointer is resolved to its
//!   role and accessible name, and only falls back to a CSS selector when the
//!   element has no name worth matching.
//! * **A password is never recorded.** Typing into a password field produces no
//!   step at all and no characters are kept, not even redacted ones: a recipe
//!   is stored in the cloud, and a "redacted" field is still a place a secret
//!   can end up.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;

use crate::wayfern_cdp::WayfernSession;

/// Emitted as each step is recognised, so the UI can show the recipe growing.
pub const EVENT_RECORDED_STEP: &str = "recipe-recording-step";
/// Emitted when a recording stops, for any reason including the browser
/// closing under it. Payload: `{ "reason": "stopped" | "browser-gone" }`.
pub const EVENT_RECORDING_ENDED: &str = "recipe-recording-ended";

/// A recording holds at most this many steps. The API refuses a longer recipe,
/// and a recording that silently kept growing would be discarded at save time.
const MAX_STEPS: usize = 200;

/// What one recording has produced so far.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingStatus {
  /// The profile being recorded, when one is.
  pub profile_id: Option<String>,
  pub steps: Vec<Value>,
  /// True while the capture is live.
  pub recording: bool,
}

#[derive(Default)]
struct RecorderState {
  profile_id: Option<String>,
  steps: Vec<Value>,
  /// Set to stop the reader task; the task also stops when the socket closes.
  cancel: Option<tokio::sync::oneshot::Sender<()>>,
  /// Text typed into the focused field since the last flush, and whether that
  /// field is a password (in which case nothing is kept).
  pending_text: String,
  pending_target: Option<Value>,
  pending_is_password: bool,
}

lazy_static::lazy_static! {
  static ref RECORDER: Arc<AsyncMutex<RecorderState>> = Arc::new(AsyncMutex::new(RecorderState::default()));
}

fn err(code: &str) -> String {
  json!({ "code": code }).to_string()
}

/// The locator a recorded step should carry for `node`, or a CSS selector when
/// the element has no name to match on.
///
/// Role and name come from the accessibility tree, which is what the resolver
/// on the other side matches against, so a step recorded here resolves there.
pub fn target_from_description(
  role: Option<&str>,
  name: Option<&str>,
  selector: Option<&str>,
) -> Option<Value> {
  let role = role.map(str::trim).filter(|role| {
    !role.is_empty() && *role != "none" && *role != "generic" && *role != "GenericContainer"
  });
  let name = name
    .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
    .filter(|name| !name.is_empty() && name.chars().count() <= 500);

  if let Some(name) = name {
    let mut locator = serde_json::Map::new();
    if let Some(role) = role {
      locator.insert("role".to_string(), json!(role));
    }
    locator.insert("name".to_string(), json!(name));
    return Some(json!({ "locator": Value::Object(locator) }));
  }
  // No accessible name: a locator on role alone would match half the page, so
  // the selector is the honest handle. Without either there is nothing to
  // record and the click is dropped rather than guessed at.
  let selector = selector.map(str::trim).filter(|s| !s.is_empty())?;
  Some(json!({ "selector": selector }))
}

/// Merge a target (`{selector}` or `{locator}`) into a step object.
fn with_target(mut step: serde_json::Map<String, Value>, target: &Value) -> Value {
  if let Some(object) = target.as_object() {
    for (key, value) in object {
      step.insert(key.clone(), value.clone());
    }
  }
  Value::Object(step)
}

/// Whether this navigation is worth a step of its own.
///
/// A click that follows a link navigates, and recording both would replay the
/// click and then jump to where it already went. Only a navigation the user
/// asked for directly — typed into the address bar, or the first page — is a
/// step, which is what `type: "typed"` and `"other"` mean in the transition.
pub fn navigation_is_user_intent(transition_type: Option<&str>) -> bool {
  matches!(
    transition_type,
    Some("typed") | Some("auto_bookmark") | Some("generated") | Some("keyword")
  )
}

/// The step a captured key sequence becomes, or `None` when nothing should be
/// recorded (a password, or no text at all).
pub fn typing_step(text: &str, target: Option<&Value>, is_password: bool) -> Option<Value> {
  if is_password || text.is_empty() {
    return None;
  }
  let target = target?;
  let mut step = serde_json::Map::new();
  step.insert("type".to_string(), json!("type"));
  step.insert("text".to_string(), json!(text));
  Some(with_target(step, target))
}

/// The step a captured click becomes.
pub fn click_step(target: &Value) -> Value {
  let mut step = serde_json::Map::new();
  step.insert("type".to_string(), json!("click"));
  with_target(step, target)
}

/// The step a user-driven navigation becomes.
pub fn navigate_step(url: &str) -> Option<Value> {
  let url = url.trim();
  if !(url.starts_with("http://") || url.starts_with("https://")) {
    return None;
  }
  Some(json!({ "type": "navigate", "url": url }))
}

/// Ask the page what sits at a viewport point, and describe it well enough to
/// build a target from.
async fn describe_point(session: &mut WayfernSession, x: f64, y: f64) -> Option<Value> {
  let node = session
    .call(
      "DOM.getNodeForLocation",
      json!({ "x": x, "y": y, "includeUserAgentShadowDOM": false }),
    )
    .await
    .ok()?;
  let backend_node_id = node.get("backendNodeId").and_then(Value::as_i64)?;

  // The accessibility view is what the resolver on the other side matches, so
  // the role and name are read from there rather than from the tag and text.
  let ax = session
    .call(
      "Accessibility.getPartialAXTree",
      json!({ "backendNodeId": backend_node_id, "fetchRelatives": false }),
    )
    .await
    .ok();
  let (role, name) = ax
    .as_ref()
    .and_then(|ax| ax.get("nodes")?.as_array()?.first().cloned())
    .map(|node| {
      (
        node["role"]["value"].as_str().map(str::to_string),
        node["name"]["value"].as_str().map(str::to_string),
      )
    })
    .unwrap_or((None, None));

  // A selector for the fallback, and the tag so a password field is known.
  let described = session
    .call(
      "DOM.describeNode",
      json!({ "backendNodeId": backend_node_id }),
    )
    .await
    .ok();
  let selector = described.as_ref().and_then(|described| {
    let node = described.get("node")?;
    let attributes = node.get("attributes")?.as_array()?;
    let mut id = None;
    let mut kind = None;
    for pair in attributes.chunks(2) {
      match (pair.first()?.as_str()?, pair.get(1)?.as_str()?) {
        ("id", value) if !value.trim().is_empty() => id = Some(value.to_string()),
        ("type", value) => kind = Some(value.to_string()),
        _ => {}
      }
    }
    let _ = kind;
    id.map(|id| format!("#{id}"))
  });

  Some(json!({
    "role": role,
    "name": name,
    "selector": selector,
    "isPassword": described
      .as_ref()
      .map(is_password_node)
      .unwrap_or(false),
  }))
}

/// Whether a described node is a password field.
pub fn is_password_node(described: &Value) -> bool {
  let Some(attributes) = described["node"]["attributes"].as_array() else {
    return false;
  };
  attributes.chunks(2).any(|pair| {
    match (
      pair.first().and_then(Value::as_str),
      pair.get(1).and_then(Value::as_str),
    ) {
      // The attribute name is the page's to spell: HTML is case-insensitive
      // here and a field spelled `TYPE` is still a password field.
      (Some(name), Some(value)) if name.eq_ignore_ascii_case("type") => {
        value.eq_ignore_ascii_case("password")
      }
      _ => false,
    }
  })
}

/// Start recording the profile's browser.
#[tauri::command]
pub async fn start_recipe_recording(
  app_handle: tauri::AppHandle,
  profile_id: String,
) -> Result<RecordingStatus, String> {
  {
    let state = RECORDER.lock().await;
    if state.profile_id.is_some() {
      return Err(err("RECORDING_ALREADY_RUNNING"));
    }
  }

  let profile = crate::profile::ProfileManager::instance()
    .list_profiles()
    .map_err(|e| format!("Failed to list profiles: {e}"))?
    .into_iter()
    .find(|p| p.id.to_string() == profile_id)
    .ok_or_else(|| crate::backend_error("PROFILE_NOT_FOUND"))?;
  if !crate::wayfern_manager::supports_wayfern_152(&profile.version) {
    return Err(err("WAYFERN_152_REQUIRED"));
  }

  let target = crate::cdp_target::resolve(&profile)
    .await
    .map_err(|_| crate::backend_error("PROFILE_NOT_RUNNING"))?;
  let mut session = WayfernSession::open(&target)
    .await
    .map_err(|e| crate::backend_error_with_detail("RECORDING_FAILED", e.to_string()))?;

  session
    .call("DOM.enable", json!({}))
    .await
    .map_err(|e| crate::backend_error_with_detail("RECORDING_FAILED", e.to_string()))?;
  let _ = session.call("Accessibility.enable", json!({})).await;
  let _ = session.call("Page.enable", json!({})).await;
  session
    .call(
      "Wayfern.enableInputCapture",
      json!({ "trackMouseMove": false }),
    )
    .await
    .map_err(|e| crate::backend_error_with_detail("RECORDING_FAILED", e.to_string()))?;

  let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
  {
    let mut state = RECORDER.lock().await;
    state.profile_id = Some(profile_id.clone());
    state.steps.clear();
    state.pending_text.clear();
    state.pending_target = None;
    state.pending_is_password = false;
    state.cancel = Some(cancel_tx);
  }

  tauri::async_runtime::spawn(async move {
    read_events(app_handle, session, cancel_rx).await;
  });

  Ok(RecordingStatus {
    profile_id: Some(profile_id),
    steps: Vec::new(),
    recording: true,
  })
}

/// Read captured input until the recording is stopped or the browser goes.
async fn read_events(
  app_handle: tauri::AppHandle,
  mut session: WayfernSession,
  mut cancel: tokio::sync::oneshot::Receiver<()>,
) {
  let reason = loop {
    let event = tokio::select! {
      _ = &mut cancel => break "stopped",
      event = session.await_any_event(
        &["Wayfern.inputCaptured", "Page.frameNavigated"],
        std::time::Duration::from_secs(3600),
      ) => event,
    };
    match event {
      Ok(Some((method, params))) => {
        handle_event(&app_handle, &mut session, &method, &params).await;
      }
      // A quiet hour is not a reason to stop; a closed socket is.
      Ok(None) => continue,
      Err(_) => break "browser-gone",
    }
  };

  let _ = session.call("Wayfern.disableInputCapture", json!({})).await;
  session.close().await;
  {
    let mut state = RECORDER.lock().await;
    if reason == "browser-gone" {
      state.profile_id = None;
    }
    state.cancel = None;
  }
  let _ = crate::events::emit(EVENT_RECORDING_ENDED, json!({ "reason": reason }));
}

async fn handle_event(
  app_handle: &tauri::AppHandle,
  session: &mut WayfernSession,
  method: &str,
  params: &Value,
) {
  let _ = app_handle;
  match method {
    "Page.frameNavigated" => {
      // Only the main frame, and only when the user asked for it.
      if params["frame"]["parentId"].is_string() {
        return;
      }
      if !navigation_is_user_intent(params["frame"]["transitionType"].as_str()) {
        return;
      }
      flush_typing().await;
      if let Some(step) = params["frame"]["url"].as_str().and_then(navigate_step) {
        push_step(step).await;
      }
    }
    "Wayfern.inputCaptured" => match params["type"].as_str() {
      Some("mousedown") => {
        let (Some(x), Some(y)) = (params["x"].as_f64(), params["y"].as_f64()) else {
          return;
        };
        if params["button"]
          .as_str()
          .is_some_and(|button| button != "left")
        {
          return;
        }
        flush_typing().await;
        let Some(described) = describe_point(session, x, y).await else {
          return;
        };
        let target = target_from_description(
          described["role"].as_str(),
          described["name"].as_str(),
          described["selector"].as_str(),
        );
        // Remember where the next keystrokes are going, and whether that field
        // is one whose characters must never be kept.
        {
          let mut state = RECORDER.lock().await;
          state.pending_target = target.clone();
          state.pending_is_password = described["isPassword"].as_bool().unwrap_or(false);
        }
        if let Some(target) = target {
          push_step(click_step(&target)).await;
        }
      }
      Some("char") => {
        let Some(text) = params["text"].as_str() else {
          return;
        };
        let mut state = RECORDER.lock().await;
        if state.pending_is_password {
          return;
        }
        if state.pending_text.chars().count() < 4000 {
          state.pending_text.push_str(text);
        }
      }
      Some("keydown") => {
        // A key that submits or moves focus ends the current field's text.
        if matches!(params["key"].as_str(), Some("Enter") | Some("Tab")) {
          flush_typing().await;
        }
      }
      _ => {}
    },
    _ => {}
  }
}

/// Turn the characters typed so far into a step, if they are worth keeping.
async fn flush_typing() {
  let step = {
    let mut state = RECORDER.lock().await;
    let text = std::mem::take(&mut state.pending_text);
    let step = typing_step(
      &text,
      state.pending_target.as_ref(),
      state.pending_is_password,
    );
    state.pending_is_password = false;
    step
  };
  if let Some(step) = step {
    push_step(step).await;
  }
}

async fn push_step(step: Value) {
  let mut state = RECORDER.lock().await;
  if state.profile_id.is_none() || state.steps.len() >= MAX_STEPS {
    return;
  }
  state.steps.push(step.clone());
  drop(state);
  let _ = crate::events::emit(EVENT_RECORDED_STEP, step);
}

/// What has been recorded so far.
#[tauri::command]
pub async fn get_recipe_recording() -> Result<RecordingStatus, String> {
  let state = RECORDER.lock().await;
  Ok(RecordingStatus {
    profile_id: state.profile_id.clone(),
    steps: state.steps.clone(),
    recording: state.profile_id.is_some() && state.cancel.is_some(),
  })
}

/// Stop recording and hand back the steps.
#[tauri::command]
pub async fn stop_recipe_recording() -> Result<RecordingStatus, String> {
  flush_typing().await;
  let (steps, profile_id) = {
    let mut state = RECORDER.lock().await;
    if let Some(cancel) = state.cancel.take() {
      let _ = cancel.send(());
    }
    let steps = std::mem::take(&mut state.steps);
    let profile_id = state.profile_id.take();
    state.pending_target = None;
    state.pending_text.clear();
    (steps, profile_id)
  };
  Ok(RecordingStatus {
    profile_id,
    steps,
    recording: false,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_click_records_what_the_resolver_can_find_again() {
    // A named element travels as a locator, which survives a different window
    // size; a coordinate would not.
    let target =
      target_from_description(Some("button"), Some("  Buy   now "), Some("#buy")).unwrap();
    assert_eq!(
      target,
      json!({ "locator": { "role": "button", "name": "Buy now" } }),
      "the accessible name is collapsed, and the selector is not needed"
    );
    assert_eq!(
      click_step(&target),
      json!({ "type": "click", "locator": { "role": "button", "name": "Buy now" } })
    );

    // No name: the selector is the only honest handle.
    assert_eq!(
      target_from_description(Some("generic"), None, Some("#cell")).unwrap(),
      json!({ "selector": "#cell" })
    );
    // A role that matches half the page is not a locator on its own.
    assert_eq!(
      target_from_description(Some("generic"), Some("   "), Some("  ")),
      None
    );
    assert_eq!(target_from_description(None, None, None), None);
  }

  #[test]
  fn a_password_is_not_recorded_at_all() {
    let target = json!({ "selector": "#pass" });
    assert_eq!(
      typing_step("hunter2", Some(&target), true),
      None,
      "not even a redacted step: a recipe is stored in the cloud"
    );
    assert_eq!(
      typing_step("hello", Some(&target), false),
      Some(json!({ "type": "type", "text": "hello", "selector": "#pass" }))
    );
    assert_eq!(typing_step("", Some(&target), false), None);
    assert_eq!(typing_step("hello", None, false), None);
  }

  #[test]
  fn a_password_field_is_recognised_from_what_the_browser_describes() {
    let password = json!({
      "node": { "attributes": ["type", "password", "name", "pw"] }
    });
    assert!(is_password_node(&password));
    let text = json!({ "node": { "attributes": ["type", "text"] } });
    assert!(!is_password_node(&text));
    assert!(!is_password_node(&json!({ "node": {} })));
    // Case is the page's choice, not a signal.
    assert!(is_password_node(&json!({
      "node": { "attributes": ["TYPE", "PASSWORD"] }
    })));
  }

  #[test]
  fn only_a_navigation_the_user_asked_for_becomes_a_step() {
    assert!(navigation_is_user_intent(Some("typed")));
    assert!(navigation_is_user_intent(Some("keyword")));
    // A link click is already recorded as the click; recording the landing too
    // would replay the click and then jump past whatever it did.
    assert!(!navigation_is_user_intent(Some("link")));
    assert!(!navigation_is_user_intent(Some("form_submit")));
    assert!(!navigation_is_user_intent(None));

    assert_eq!(
      navigate_step(" https://example.com/shop "),
      Some(json!({ "type": "navigate", "url": "https://example.com/shop" }))
    );
    // A recipe that opens a local file is not a recipe anyone should replay.
    assert_eq!(navigate_step("file:///etc/passwd"), None);
    assert_eq!(navigate_step("about:blank"), None);
  }
}
