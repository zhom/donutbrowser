use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tokio::sync::Mutex as AsyncMutex;

use crate::profile::manager::ProfileManager;
use crate::profile::types::BrowserProfile;

/// Maximum number of profiles to launch concurrently
const MAX_CONCURRENT_LAUNCHES: usize = 5;

/// Event captured from the leader browser via Wayfern.inputCaptured CDP events.
/// Fields match the Wayfern.inputCaptured event schema directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturedEvent {
  #[serde(rename = "type")]
  pub event_type: String,
  #[serde(default)]
  pub url: Option<String>,
  #[serde(default)]
  pub x: Option<f64>,
  #[serde(default)]
  pub y: Option<f64>,
  #[serde(default)]
  pub button: Option<String>,
  #[serde(default, rename = "clickCount")]
  pub click_count: Option<i32>,
  #[serde(default)]
  pub key: Option<String>,
  #[serde(default)]
  pub code: Option<String>,
  #[serde(default, rename = "windowsVirtualKeyCode")]
  pub key_code: Option<i32>,
  #[serde(default)]
  pub modifiers: Option<i32>,
  #[serde(default)]
  pub text: Option<String>,
  #[serde(default, rename = "deltaX")]
  pub delta_x: Option<f64>,
  #[serde(default, rename = "deltaY")]
  pub delta_y: Option<f64>,
  #[serde(default)]
  pub timestamp: Option<f64>,
}

// No JavaScript injection needed — Wayfern.enableInputCapture handles everything natively.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncFollowerState {
  pub profile_id: String,
  pub profile_name: String,
  /// None = healthy, Some(url) = desynced at this URL
  pub failed_at_url: Option<String>,
  /// Held out of the mirroring on purpose. The window stays open and usable;
  /// it simply stops receiving what the leader does until it is brought back.
  #[serde(default)]
  pub held: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSessionInfo {
  pub id: String,
  pub leader_profile_id: String,
  pub leader_profile_name: String,
  pub followers: Vec<SyncFollowerState>,
  /// Mirroring is suspended for the whole session, so the leader can be used
  /// on its own without every follower copying it.
  #[serde(default)]
  pub paused: bool,
}

/// How the follower windows are placed on the host display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowLayout {
  /// Even tiles, as square a grid as the count allows. Never overlapping.
  Grid,
  /// One full-height column per window, left to right.
  Columns,
  /// Stacked with a fixed offset, so every title bar stays reachable.
  Cascade,
}

/// A window position in the host display's CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct WindowRect {
  pub left: i32,
  pub top: i32,
  pub width: u32,
  pub height: u32,
}

/// Smallest window an arrangement will ask for. Below this a browser window is
/// not usable, and Chromium clamps it anyway, which would break the "no
/// overlap" promise of the grid without anyone noticing.
const MIN_WINDOW_WIDTH: u32 = 240;
const MIN_WINDOW_HEIGHT: u32 = 180;

/// Cascade offsets, capped so a long session does not walk the last window off
/// the bottom of the display.
const CASCADE_MAX_STEP_X: u32 = 48;
const CASCADE_MAX_STEP_Y: u32 = 40;

/// Where each of `count` follower windows goes on a `screen_width` by
/// `screen_height` display.
///
/// Pure arithmetic on purpose: it is the part that can be wrong in a way no
/// screenshot would show, and it is the part a test can hold to "every window
/// is on the display, and the grid never overlaps".
pub fn layout_windows(
  layout: WindowLayout,
  count: usize,
  screen_width: u32,
  screen_height: u32,
) -> Vec<WindowRect> {
  if count == 0 || screen_width == 0 || screen_height == 0 {
    return Vec::new();
  }

  match layout {
    WindowLayout::Grid => {
      let columns = (count as f64).sqrt().ceil() as usize;
      let rows = count.div_ceil(columns);
      let cell_width = (screen_width / columns as u32).max(1);
      let cell_height = (screen_height / rows as u32).max(1);
      (0..count)
        .map(|index| {
          let column = index % columns;
          let row = index / columns;
          WindowRect {
            left: (column as u32 * cell_width) as i32,
            top: (row as u32 * cell_height) as i32,
            width: cell_width,
            height: cell_height,
          }
        })
        .collect()
    }
    WindowLayout::Columns => {
      let width = (screen_width / count as u32).max(1);
      (0..count)
        .map(|index| WindowRect {
          left: (index as u32 * width) as i32,
          top: 0,
          width,
          height: screen_height,
        })
        .collect()
    }
    WindowLayout::Cascade => {
      let width = (screen_width * 2 / 3).clamp(1, screen_width);
      let height = (screen_height * 2 / 3).clamp(1, screen_height);
      let steps = count.saturating_sub(1) as u32;
      // Spread the leftover room over the gaps, then cap it, so the last
      // window lands inside the display however many there are.
      let step_x = (screen_width - width)
        .checked_div(steps)
        .map_or(0, |step| step.min(CASCADE_MAX_STEP_X));
      let step_y = (screen_height - height)
        .checked_div(steps)
        .map_or(0, |step| step.min(CASCADE_MAX_STEP_Y));
      (0..count)
        .map(|index| WindowRect {
          left: (index as u32 * step_x) as i32,
          top: (index as u32 * step_y) as i32,
          width,
          height,
        })
        .collect()
    }
  }
}

/// Whether an arrangement is worth asking a browser for. A display too small
/// to give every window a usable size is better left alone than tiled into
/// slivers Chromium will silently refuse to make.
pub fn layout_fits(rects: &[WindowRect]) -> bool {
  rects
    .iter()
    .all(|rect| rect.width >= MIN_WINDOW_WIDTH && rect.height >= MIN_WINDOW_HEIGHT)
}

/// What the leader's events are currently allowed to reach.
///
/// Kept apart from the session record so the hot event path can read it
/// without an async lock, and so the rules are testable on their own.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct MirrorGate {
  paused: bool,
  held: std::collections::HashSet<String>,
}

impl MirrorGate {
  /// True when this follower should receive what the leader just did.
  pub(crate) fn accepts(&self, follower_id: &str) -> bool {
    !self.paused && !self.held.contains(follower_id)
  }

  pub(crate) fn set_paused(&mut self, paused: bool) {
    self.paused = paused;
  }

  pub(crate) fn is_paused(&self) -> bool {
    self.paused
  }

  pub(crate) fn set_held(&mut self, follower_id: &str, held: bool) {
    if held {
      self.held.insert(follower_id.to_string());
    } else {
      self.held.remove(follower_id);
    }
  }

  pub(crate) fn is_held(&self, follower_id: &str) -> bool {
    self.held.contains(follower_id)
  }

  /// Drop a follower that has left the session, so its id cannot linger and
  /// silence a profile that is later added back.
  pub(crate) fn forget(&mut self, follower_id: &str) {
    self.held.remove(follower_id);
  }
}

type SharedGate = Arc<std::sync::RwLock<MirrorGate>>;

/// Everything the listener task needs to know about the session it drives.
struct SessionLoopContext {
  session_id: String,
  leader_profile_id: String,
  follower_profile_ids: Vec<String>,
  gate: SharedGate,
}

/// Internal session state
struct SyncSession {
  id: String,
  leader_profile_id: String,
  leader_profile_name: String,
  /// Ordered, because the arrangement numbers the windows by it and a list
  /// that reshuffles itself between renders is unusable.
  followers: Vec<SyncFollowerState>,
  gate: SharedGate,
  /// Cancellation token — drop sender to stop the listener task
  cancel_tx: tokio::sync::watch::Sender<bool>,
}

impl SyncSession {
  /// The session as the page sees it.
  ///
  /// `paused` and every `held` flag are read back off the gate rather than
  /// mirrored into the follower records, so what the panel shows is what the
  /// event path actually enforces and the two can never drift apart.
  fn info(&self) -> SyncSessionInfo {
    let gate = self.gate.read().ok();
    SyncSessionInfo {
      id: self.id.clone(),
      leader_profile_id: self.leader_profile_id.clone(),
      leader_profile_name: self.leader_profile_name.clone(),
      followers: self
        .followers
        .iter()
        .map(|follower| SyncFollowerState {
          held: gate
            .as_ref()
            .is_some_and(|gate| gate.is_held(&follower.profile_id)),
          ..follower.clone()
        })
        .collect(),
      paused: gate.as_ref().is_some_and(|gate| gate.is_paused()),
    }
  }

  fn follower_mut(&mut self, profile_id: &str) -> Option<&mut SyncFollowerState> {
    self
      .followers
      .iter_mut()
      .find(|follower| follower.profile_id == profile_id)
  }

  fn has_follower(&self, profile_id: &str) -> bool {
    self
      .followers
      .iter()
      .any(|follower| follower.profile_id == profile_id)
  }
}

pub struct SynchronizerManager {
  inner: Arc<AsyncMutex<SynchronizerInner>>,
}

struct SynchronizerInner {
  sessions: HashMap<String, SyncSession>,
}

static SYNCHRONIZER: std::sync::OnceLock<SynchronizerManager> = std::sync::OnceLock::new();

impl SynchronizerManager {
  pub fn instance() -> &'static SynchronizerManager {
    SYNCHRONIZER.get_or_init(|| SynchronizerManager {
      inner: Arc::new(AsyncMutex::new(SynchronizerInner {
        sessions: HashMap::new(),
      })),
    })
  }

  /// Start a new sync session. Launches all profiles and begins event capture.
  pub async fn start_session(
    &self,
    app_handle: tauri::AppHandle,
    leader_profile_id: String,
    follower_profile_ids: Vec<String>,
  ) -> Result<SyncSessionInfo, String> {
    // Validate: leader must be wayfern
    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;

    let leader = profiles
      .iter()
      .find(|p| p.id.to_string() == leader_profile_id)
      .ok_or("Leader profile not found")?
      .clone();

    if leader.browser != "wayfern" {
      return Err("Synchronizer only supports Wayfern profiles.".to_string());
    }

    // Check leader is not already running
    if leader.process_id.is_some() {
      let sys = sysinfo::System::new_all();
      if let Some(pid) = leader.process_id {
        if sys.process(sysinfo::Pid::from(pid as usize)).is_some() {
          return Err(
            "Leader profile is already running. Stop it first to start a sync session.".to_string(),
          );
        }
      }
    }

    let mut follower_profiles: Vec<BrowserProfile> = Vec::new();
    for fid in &follower_profile_ids {
      let fp = profiles
        .iter()
        .find(|p| p.id.to_string() == *fid)
        .ok_or(format!("Follower profile '{fid}' not found"))?
        .clone();
      if fp.browser != "wayfern" {
        return Err(format!(
          "Profile '{}' is not a Wayfern profile. Only Wayfern profiles can be synchronized.",
          fp.name
        ));
      }
      follower_profiles.push(fp);
    }

    // Check no profile is part of another active session
    {
      let inner = self.inner.lock().await;
      for session in inner.sessions.values() {
        if session.leader_profile_id == leader_profile_id {
          return Err("Leader profile is already in another sync session.".to_string());
        }
        for fid in &follower_profile_ids {
          if session.leader_profile_id == *fid || session.has_follower(fid) {
            return Err(format!(
              "Profile '{fid}' is already part of another sync session."
            ));
          }
        }
      }
    }

    let session_id = uuid::Uuid::new_v4().to_string();

    log::info!(
      "Synchronizer: launching leader '{}' and {} followers",
      leader.name,
      follower_profiles.len()
    );

    // Launch leader first so it gets focus
    crate::browser_runner::launch_browser_profile(app_handle.clone(), leader.clone(), None, None)
      .await
      .map_err(|e| format!("Failed to launch leader: {e}"))?;

    // Launch followers in parallel batches of MAX_CONCURRENT_LAUNCHES
    for chunk in follower_profiles.chunks(MAX_CONCURRENT_LAUNCHES) {
      let mut set = tokio::task::JoinSet::new();
      for fp in chunk {
        let ah = app_handle.clone();
        let fp = fp.clone();
        set.spawn(async move {
          crate::browser_runner::launch_browser_profile(ah, fp.clone(), None, None)
            .await
            .map_err(|e| (fp.name.clone(), e.to_string()))
        });
      }
      while let Some(result) = set.join_next().await {
        match result {
          Ok(Ok(_)) => {}
          Ok(Err((name, e))) => {
            log::error!("Failed to launch follower '{name}': {e}");
            // Kill leader and all already-launched followers
            let _ =
              crate::browser_runner::kill_browser_profile(app_handle.clone(), leader.clone()).await;
            for fp in &follower_profiles {
              let _ =
                crate::browser_runner::kill_browser_profile(app_handle.clone(), fp.clone()).await;
            }
            return Err(format!("Failed to launch follower '{name}': {e}"));
          }
          Err(e) => {
            log::error!("Launch task panicked: {e}");
            let _ =
              crate::browser_runner::kill_browser_profile(app_handle.clone(), leader.clone()).await;
            return Err(format!("Launch task panicked: {e}"));
          }
        }
      }
    }

    // Bring leader window to front after all followers launched
    Self::focus_leader_window(&leader).await;

    // Build follower states, keeping the order the user selected them in.
    let followers: Vec<SyncFollowerState> = follower_profiles
      .iter()
      .map(|fp| SyncFollowerState {
        profile_id: fp.id.to_string(),
        profile_name: fp.name.clone(),
        failed_at_url: None,
        held: false,
      })
      .collect();

    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let gate: SharedGate = Arc::new(std::sync::RwLock::new(MirrorGate::default()));

    let session = SyncSession {
      id: session_id.clone(),
      leader_profile_id: leader_profile_id.clone(),
      leader_profile_name: leader.name.clone(),
      followers: followers.clone(),
      gate: gate.clone(),
      cancel_tx,
    };

    let info = session.info();

    {
      let mut inner = self.inner.lock().await;
      inner.sessions.insert(session_id.clone(), session);
    }

    // Emit initial session event
    let _ = app_handle.emit("sync-session-changed", &info);

    // Spawn the CDP listener task with a readiness signal
    let manager = self.inner.clone();
    let ah = app_handle.clone();
    let sid = session_id.clone();
    let lid = leader_profile_id.clone();
    let fids: Vec<String> = follower_profile_ids.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();

    let all_profile_ids: Vec<String> = std::iter::once(leader_profile_id.clone())
      .chain(follower_profile_ids.iter().cloned())
      .collect();

    log::info!("Synchronizer: spawning CDP listener task");

    tokio::spawn(async move {
      log::info!("Synchronizer: CDP listener task started");
      if let Err(e) = Self::run_session_loop(
        ah.clone(),
        manager.clone(),
        SessionLoopContext {
          session_id: sid.clone(),
          leader_profile_id: lid,
          follower_profile_ids: fids,
          gate,
        },
        cancel_rx,
        ready_tx,
      )
      .await
      {
        log::error!("Synchronizer session {sid} error: {e}");
        // Kill all profiles on error (leader + followers)
        for pid in &all_profile_ids {
          if let Ok(p) = Self::get_profile(pid) {
            let _ = crate::browser_runner::kill_browser_profile(ah.clone(), p).await;
          }
        }
      }
      // Session ended — clean up
      let mut inner = manager.lock().await;
      inner.sessions.remove(&sid);
      let _ = ah.emit("sync-session-ended", &sid);
    });

    // Wait for the CDP session to be ready (or fail)
    match tokio::time::timeout(std::time::Duration::from_secs(90), ready_rx).await {
      Ok(Ok(Ok(()))) => Ok(info),
      Ok(Ok(Err(e))) => Err(format!("Synchronizer setup failed: {e}")),
      Ok(Err(_)) => Err("Synchronizer setup channel closed unexpectedly".to_string()),
      Err(_) => Err("Synchronizer setup timed out".to_string()),
    }
  }

  /// Bring the leader browser window to front.
  ///
  /// On macOS this is a no-op on purpose: the only way to raise another
  /// app's window from Rust is via `osascript` / Apple Events, which
  /// triggers the TCC "prevented from modifying other apps" prompt. Donut
  /// must never touch other apps on the user's Mac.
  async fn focus_leader_window(leader: &BrowserProfile) {
    let profile = match Self::get_profile(&leader.id.to_string()) {
      Ok(p) => p,
      Err(_) => return,
    };
    let Some(pid) = profile.process_id else {
      return;
    };

    #[cfg(target_os = "linux")]
    {
      let _ = tokio::process::Command::new("xdotool")
        .args([
          "search",
          "--pid",
          &pid.to_string(),
          "--onlyvisible",
          "windowactivate",
        ])
        .output()
        .await;
    }

    #[cfg(not(target_os = "linux"))]
    {
      let _ = pid;
    }
  }

  /// Core session loop: inject capture script on leader, listen for events, replay on followers.
  async fn run_session_loop(
    app_handle: tauri::AppHandle,
    manager: Arc<AsyncMutex<SynchronizerInner>>,
    session: SessionLoopContext,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    ready_tx: tokio::sync::oneshot::Sender<Result<(), String>>,
  ) -> Result<(), String> {
    let SessionLoopContext {
      session_id,
      leader_profile_id,
      follower_profile_ids,
      gate,
    } = session;
    use futures_util::sink::SinkExt;
    use futures_util::stream::StreamExt;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message;

    log::info!("Synchronizer: run_session_loop started, waiting 1s for browsers");
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Connect to leader page-level target for reliable event capture
    log::info!("Synchronizer: getting leader CDP port");
    let leader_profile = Self::get_profile(&leader_profile_id)?;
    let leader_port = Self::get_cdp_port(&leader_profile).await?;
    log::info!("Synchronizer: leader CDP port = {leader_port}, getting WS URL");
    let leader_ws_url = Self::get_page_ws_url(leader_port).await?;

    log::info!("Synchronizer: connecting to leader page");

    let (mut ws_stream, _) = connect_async(&leader_ws_url)
      .await
      .map_err(|e| format!("Failed to connect to leader CDP: {e}"))?;

    // Helper: send command and collect response, buffering non-response events
    let mut cmd_id: u64 = 0;
    let mut pending_events: Vec<serde_json::Value> = Vec::new();

    // Send a CDP command and wait for its response, buffering events that arrive in between
    async fn send_cmd(
      ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
      >,
      cmd_id: &mut u64,
      pending_events: &mut Vec<serde_json::Value>,
      method: &str,
      params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
      *cmd_id += 1;
      let id = *cmd_id;
      let cmd = serde_json::json!({ "id": id, "method": method, "params": params });
      ws.send(Message::Text(cmd.to_string().into()))
        .await
        .map_err(|e| format!("Failed to send {method}: {e}"))?;
      let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(10);
      loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
          return Err(format!("Timeout waiting for {method} response"));
        }
        match tokio::time::timeout(remaining, ws.next()).await {
          Ok(Some(Ok(Message::Text(text)))) => {
            let resp: serde_json::Value = serde_json::from_str(text.as_str()).unwrap_or_default();
            if resp.get("id") == Some(&serde_json::json!(id)) {
              if let Some(error) = resp.get("error") {
                return Err(format!("CDP error for {method}: {error}"));
              }
              return Ok(resp.get("result").cloned().unwrap_or(serde_json::json!({})));
            }
            // Buffer events that arrive while waiting for response
            if resp.get("method").is_some() {
              pending_events.push(resp);
            }
          }
          Ok(Some(Ok(_))) => continue,
          Ok(Some(Err(e))) => return Err(format!("WebSocket error: {e}")),
          Ok(None) => return Err("WebSocket closed".to_string()),
          Err(_) => return Err(format!("Timeout waiting for {method} response")),
        }
      }
    }

    // Use Wayfern's native input capture — no JS injection needed.
    // Captures all real user input at the browser process level.
    let setup_commands: Vec<(&str, serde_json::Value)> = vec![
      ("Page.enable", serde_json::json!({})),
      ("Wayfern.enableInputCapture", serde_json::json!({})),
    ];

    for (method, params) in setup_commands {
      match send_cmd(
        &mut ws_stream,
        &mut cmd_id,
        &mut pending_events,
        method,
        params,
      )
      .await
      {
        Ok(_) => log::info!("Synchronizer: {method} OK"),
        Err(e) => {
          log::error!("Synchronizer: {method} FAILED: {e}");
          return Err(format!("{method} failed: {e}"));
        }
      }
    }

    log::info!("Synchronizer: input capture enabled");

    // Get leader window size and resize all followers to match
    let leader_bounds = send_cmd(
      &mut ws_stream,
      &mut cmd_id,
      &mut pending_events,
      "Browser.getWindowForTarget",
      serde_json::json!({}),
    )
    .await
    .ok();

    if let Some(bounds_result) = &leader_bounds {
      if let Some(bounds) = bounds_result.get("bounds") {
        let width = bounds.get("width").and_then(|v| v.as_i64()).unwrap_or(0);
        let height = bounds.get("height").and_then(|v| v.as_i64()).unwrap_or(0);
        if width > 0 && height > 0 {
          log::info!("Synchronizer: leader window size {width}x{height}, resizing followers");
          for fid in &follower_profile_ids {
            if let Ok(fp) = Self::get_profile(fid) {
              if let Ok(port) = Self::get_cdp_port(&fp).await {
                if let Ok(f_ws) = Self::get_page_ws_url(port).await {
                  if let Ok((mut fws, _)) = tokio_tungstenite::connect_async(&f_ws).await {
                    // Get follower's window ID
                    let get_win = serde_json::json!({ "id": 1, "method": "Browser.getWindowForTarget", "params": {} });
                    let _ = fws.send(Message::Text(get_win.to_string().into())).await;
                    if let Some(Ok(Message::Text(text))) = fws.next().await {
                      if let Ok(resp) = serde_json::from_str::<serde_json::Value>(text.as_str()) {
                        if let Some(win_id) = resp
                          .get("result")
                          .and_then(|r| r.get("windowId"))
                          .and_then(|w| w.as_i64())
                        {
                          let set_bounds = serde_json::json!({
                            "id": 2,
                            "method": "Browser.setWindowBounds",
                            "params": { "windowId": win_id, "bounds": { "width": width, "height": height } }
                          });
                          let _ = fws.send(Message::Text(set_bounds.to_string().into())).await;
                        }
                      }
                    }
                  }
                }
              }
            }
          }
        }
      }
    }

    log::info!("Synchronizer: opening persistent connections to followers");

    // Open persistent WebSocket connections to each follower and create event channels.
    // Each follower gets a dedicated replay task with a long-lived WS connection.
    let mut follower_senders: HashMap<String, tokio::sync::mpsc::UnboundedSender<CapturedEvent>> =
      HashMap::new();

    for fid in &follower_profile_ids {
      match Self::get_profile(fid) {
        Ok(fp) => match Self::get_cdp_port(&fp).await {
          Ok(port) => match Self::get_page_ws_url(port).await {
            Ok(url) => {
              match tokio_tungstenite::connect_async(&url).await {
                Ok((ws, _)) => {
                  log::info!("Synchronizer: follower connected");
                  let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<CapturedEvent>();
                  follower_senders.insert(fid.clone(), tx);

                  // Spawn dedicated replay task for this follower
                  let fid_clone = fid.clone();
                  let mgr = manager.clone();
                  let sid = session_id.clone();
                  let ah = app_handle.clone();
                  tokio::spawn(async move {
                    Self::follower_replay_loop(ws, rx, fid_clone, mgr, sid, ah).await;
                  });
                }
                Err(e) => log::warn!(
                  "Synchronizer: failed to connect to follower {}: {e}",
                  fp.name
                ),
              }
            }
            Err(e) => log::warn!(
              "Synchronizer: failed to get WS URL for follower {}: {e}",
              fp.name
            ),
          },
          Err(e) => log::warn!(
            "Synchronizer: failed to get CDP port for follower {}: {e}",
            fp.name
          ),
        },
        Err(e) => log::warn!("Synchronizer: failed to get follower profile {fid}: {e}"),
      }
    }

    log::info!(
      "Synchronizer: {} of {} followers connected",
      follower_senders.len(),
      follower_profile_ids.len()
    );

    // Track when the last user interaction was captured (for suppressing click-caused nav replay)
    let mut last_user_event_time = std::time::Instant::now() - std::time::Duration::from_secs(60);

    // Signal that the session is ready for interaction
    let _ = ready_tx.send(Ok(()));

    // Process any events that were buffered during setup
    for event in pending_events.drain(..) {
      Self::handle_cdp_event(&event, &gate, &follower_senders, false).await;
    }

    // Main event loop — listen for Wayfern.inputCaptured events
    loop {
      tokio::select! {
          _ = cancel_rx.changed() => {
              if *cancel_rx.borrow() {
                  log::info!("Synchronizer session {session_id}: cancelled");
                  break;
              }
          }
          msg = ws_stream.next() => {
              match msg {
                  Some(Ok(Message::Text(text))) => {
                      let value: serde_json::Value = match serde_json::from_str(text.as_str()) {
                          Ok(v) => v,
                          Err(_) => continue,
                      };

                      let method = value.get("method").and_then(|m| m.as_str()).unwrap_or("");

                      // Log CDP command response errors
                      if let Some(id) = value.get("id") {
                          if let Some(error) = value.get("error") {
                              log::warn!("Synchronizer: CDP command {id} error: {error}");
                          }
                      }

                      // Track user interaction timing
                      if method == "Wayfern.inputCaptured" {
                          last_user_event_time = std::time::Instant::now();
                      }

                      let recent_user_event = last_user_event_time.elapsed() < std::time::Duration::from_secs(2);

                      Self::handle_cdp_event(
                          &value,
                          &gate,
                          &follower_senders,
                          recent_user_event,
                      ).await;
                  }
                  Some(Ok(_)) => {} // pings, binary, etc.
                  Some(Err(e)) => {
                      log::error!("Synchronizer: leader WebSocket error: {e}");
                      break;
                  }
                  None => {
                      log::info!("Synchronizer: leader WebSocket closed (browser closed)");
                      break;
                  }
              }
          }
      }
    }

    // Leader closed or session cancelled — kill all followers
    log::info!("Synchronizer session {session_id}: stopping all followers");
    let follower_ids: Vec<String> = {
      let inner = manager.lock().await;
      if let Some(session) = inner.sessions.get(&session_id) {
        session
          .followers
          .iter()
          .map(|follower| follower.profile_id.clone())
          .collect()
      } else {
        Vec::new()
      }
    };

    for fid in follower_ids {
      if let Ok(fp) = Self::get_profile(&fid) {
        let _ = crate::browser_runner::kill_browser_profile(app_handle.clone(), fp).await;
      }
    }

    Ok(())
  }

  /// Handle a single CDP event from the leader.
  ///
  /// The gate is read here rather than in the per-follower replay task so a
  /// paused session queues nothing at all: an event dropped now cannot arrive
  /// late once mirroring resumes.
  async fn handle_cdp_event(
    value: &serde_json::Value,
    gate: &SharedGate,
    follower_senders: &HashMap<String, tokio::sync::mpsc::UnboundedSender<CapturedEvent>>,
    recent_user_event: bool,
  ) {
    let method = value.get("method").and_then(|m| m.as_str()).unwrap_or("");

    // Handle Wayfern.inputCaptured — native input events from the browser process
    if method == "Wayfern.inputCaptured" {
      if let Some(params) = value.get("params") {
        let event_type = params.get("type").and_then(|v| v.as_str()).unwrap_or("");
        // Skip mousemove — too noisy and not useful for synchronization
        if event_type == "mousemove" {
          return;
        }
        if let Ok(event) = serde_json::from_value::<CapturedEvent>(params.clone()) {
          log::info!("Synchronizer: captured {event_type}");
          Self::fan_out(gate, follower_senders, &event);
        }
      }
    }

    // Handle Page.frameNavigated — replay only for address-bar navigations
    if method == "Page.frameNavigated" && !recent_user_event {
      if let Some(params) = value.get("params") {
        if let Some(frame) = params.get("frame") {
          let is_top = frame.get("parentId").is_none();
          if is_top {
            if let Some(url) = frame.get("url").and_then(|v| v.as_str()) {
              if !url.starts_with("about:") && !url.starts_with("chrome://") {
                log::info!("Synchronizer: replaying address-bar navigation");
                let nav_event = CapturedEvent {
                  event_type: "navigate".to_string(),
                  url: Some(url.to_string()),
                  x: None,
                  y: None,
                  button: None,
                  click_count: None,
                  key: None,
                  code: None,
                  key_code: None,
                  modifiers: None,
                  text: None,
                  delta_x: None,
                  delta_y: None,
                  timestamp: None,
                };
                Self::fan_out(gate, follower_senders, &nav_event);
              }
            }
          }
        }
      }
    }
  }

  /// Send one captured event to every follower the gate still admits.
  ///
  /// The lock is held across the whole fan-out and released before any await,
  /// so a pause taking effect mid-event cannot deliver to half the followers.
  fn fan_out(
    gate: &SharedGate,
    follower_senders: &HashMap<String, tokio::sync::mpsc::UnboundedSender<CapturedEvent>>,
    event: &CapturedEvent,
  ) {
    let Ok(gate) = gate.read() else {
      log::warn!("Synchronizer: mirroring gate is poisoned; dropping the event");
      return;
    };
    if gate.is_paused() {
      return;
    }
    for (follower_id, tx) in follower_senders {
      if !gate.accepts(follower_id) {
        continue;
      }
      let _ = tx.send(event.clone());
    }
  }

  /// Dedicated replay loop for a single follower with a persistent WebSocket connection.
  /// Processes events from the channel sequentially — no per-event connection overhead.
  async fn follower_replay_loop(
    mut ws: tokio_tungstenite::WebSocketStream<
      tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<CapturedEvent>,
    follower_id: String,
    manager: Arc<AsyncMutex<SynchronizerInner>>,
    session_id: String,
    app_handle: tauri::AppHandle,
  ) {
    use futures_util::sink::SinkExt;
    use tokio_tungstenite::tungstenite::Message;

    let mut cmd_id: u64 = 0;

    while let Some(event) = rx.recv().await {
      cmd_id += 1;
      let button = event.button.clone().unwrap_or_else(|| "left".to_string());

      let command = match event.event_type.as_str() {
        "navigate" => event
          .url
          .as_ref()
          .map(|url| ("Page.navigate", serde_json::json!({ "url": url }))),
        "mousedown" => Some((
          "Input.dispatchMouseEvent",
          serde_json::json!({
            "type": "mousePressed",
            "x": event.x.unwrap_or(0.0),
            "y": event.y.unwrap_or(0.0),
            "button": button,
            "clickCount": event.click_count.unwrap_or(1),
            "modifiers": event.modifiers.unwrap_or(0),
          }),
        )),
        "mouseup" => Some((
          "Input.dispatchMouseEvent",
          serde_json::json!({
            "type": "mouseReleased",
            "x": event.x.unwrap_or(0.0),
            "y": event.y.unwrap_or(0.0),
            "button": button,
            "modifiers": event.modifiers.unwrap_or(0),
          }),
        )),
        "keydown" => Some((
          "Input.dispatchKeyEvent",
          serde_json::json!({
            "type": "keyDown",
            "key": event.key.clone().unwrap_or_default(),
            "code": event.code.clone().unwrap_or_default(),
            "windowsVirtualKeyCode": event.key_code.unwrap_or(0),
            "modifiers": event.modifiers.unwrap_or(0),
          }),
        )),
        "keyup" => Some((
          "Input.dispatchKeyEvent",
          serde_json::json!({
            "type": "keyUp",
            "key": event.key.clone().unwrap_or_default(),
            "code": event.code.clone().unwrap_or_default(),
            "windowsVirtualKeyCode": event.key_code.unwrap_or(0),
            "modifiers": event.modifiers.unwrap_or(0),
          }),
        )),
        "char" => {
          let text = event.text.clone().unwrap_or_default();
          if text.is_empty() {
            None
          } else {
            Some((
              "Input.dispatchKeyEvent",
              serde_json::json!({
                "type": "char",
                "text": text,
                "unmodifiedText": text,
                "modifiers": event.modifiers.unwrap_or(0),
              }),
            ))
          }
        }
        "wheel" => {
          let dx = -event.delta_x.unwrap_or(0.0);
          let dy = -event.delta_y.unwrap_or(0.0);
          Some((
            "Runtime.evaluate",
            serde_json::json!({
              "expression": format!("window.scrollBy({dx},{dy})"),
            }),
          ))
        }
        _ => None,
      };

      if let Some((method, params)) = command {
        let cmd = serde_json::json!({ "id": cmd_id, "method": method, "params": params });
        if let Err(e) = ws.send(Message::Text(cmd.to_string().into())).await {
          log::warn!("Synchronizer: follower {follower_id} send failed: {e}");
          // Mark as desynced
          let mut inner = manager.lock().await;
          if let Some(session) = inner.sessions.get_mut(&session_id) {
            if let Some(follower) = session.follower_mut(&follower_id) {
              follower.failed_at_url = Some("connection lost".to_string());
              let info = session.info();
              let _ = app_handle.emit("sync-session-changed", &info);
            }
          }
          break;
        }
        // Don't wait for response — fire and forget for speed.
        // CDP commands are processed in order by Chromium.
      }
    }
  }

  /// Stop a sync session by ID. Kills all followers.
  pub async fn stop_session(
    &self,
    app_handle: tauri::AppHandle,
    session_id: &str,
  ) -> Result<(), String> {
    let mut inner = self.inner.lock().await;
    let session = inner
      .sessions
      .remove(session_id)
      .ok_or("Session not found")?;

    // Signal the listener task to stop
    let _ = session.cancel_tx.send(true);

    // Kill followers
    for follower in &session.followers {
      if let Ok(fp) = Self::get_profile(&follower.profile_id) {
        let _ = crate::browser_runner::kill_browser_profile(app_handle.clone(), fp).await;
      }
    }

    // Kill leader
    if let Ok(leader) = Self::get_profile(&session.leader_profile_id) {
      let _ = crate::browser_runner::kill_browser_profile(app_handle.clone(), leader).await;
    }

    let _ = app_handle.emit("sync-session-ended", session_id);
    Ok(())
  }

  /// Remove a single follower from an active session (user clicked stop on follower).
  pub async fn remove_follower(
    &self,
    app_handle: tauri::AppHandle,
    session_id: &str,
    follower_profile_id: &str,
  ) -> Result<(), String> {
    let mut inner = self.inner.lock().await;
    let session = inner
      .sessions
      .get_mut(session_id)
      .ok_or("Session not found")?;

    session
      .followers
      .retain(|follower| follower.profile_id != follower_profile_id);
    // A removed follower must not leave its id sitting in the held set: the
    // profile could be a follower again in a later session and would then be
    // silently ignored.
    if let Ok(mut gate) = session.gate.write() {
      gate.forget(follower_profile_id);
    }

    // Kill the follower browser
    if let Ok(fp) = Self::get_profile(follower_profile_id) {
      let _ = crate::browser_runner::kill_browser_profile(app_handle.clone(), fp).await;
    }

    // Emit updated session info
    let info = session.info();
    let _ = app_handle.emit("sync-session-changed", &info);

    Ok(())
  }

  /// Suspend or resume mirroring for a whole session.
  ///
  /// Nothing is queued while it is paused: the leader can be used on its own
  /// and the followers stay exactly where they were, rather than replaying a
  /// backlog the moment mirroring comes back.
  pub async fn set_paused(
    &self,
    app_handle: tauri::AppHandle,
    session_id: &str,
    paused: bool,
  ) -> Result<SyncSessionInfo, String> {
    let mut inner = self.inner.lock().await;
    let session = inner
      .sessions
      .get_mut(session_id)
      .ok_or_else(|| serde_json::json!({ "code": "SYNC_SESSION_NOT_FOUND" }).to_string())?;

    session
      .gate
      .write()
      .map_err(|_| serde_json::json!({ "code": "SYNC_SESSION_UNAVAILABLE" }).to_string())?
      .set_paused(paused);

    let info = session.info();
    let _ = app_handle.emit("sync-session-changed", &info);
    log::info!(
      "Synchronizer session {session_id}: mirroring {}",
      if paused { "paused" } else { "resumed" }
    );
    Ok(info)
  }

  /// Hold one follower out of the mirroring, or bring it back.
  ///
  /// Unlike removing a follower this keeps the browser open, so a person can
  /// do something in that one window and then rejoin it to the session.
  pub async fn set_follower_held(
    &self,
    app_handle: tauri::AppHandle,
    session_id: &str,
    follower_profile_id: &str,
    held: bool,
  ) -> Result<SyncSessionInfo, String> {
    let mut inner = self.inner.lock().await;
    let session = inner
      .sessions
      .get_mut(session_id)
      .ok_or_else(|| serde_json::json!({ "code": "SYNC_SESSION_NOT_FOUND" }).to_string())?;

    if !session.has_follower(follower_profile_id) {
      return Err(serde_json::json!({ "code": "SYNC_FOLLOWER_NOT_FOUND" }).to_string());
    }

    session
      .gate
      .write()
      .map_err(|_| serde_json::json!({ "code": "SYNC_SESSION_UNAVAILABLE" }).to_string())?
      .set_held(follower_profile_id, held);

    let info = session.info();
    let _ = app_handle.emit("sync-session-changed", &info);
    Ok(info)
  }

  /// Place the session's follower windows on the host display.
  ///
  /// Held-out followers are placed too: they are still windows on the screen,
  /// and a person who held one out is usually the one who wants to see it.
  pub async fn arrange_windows(
    &self,
    app_handle: tauri::AppHandle,
    session_id: &str,
    layout: WindowLayout,
  ) -> Result<SyncSessionInfo, String> {
    let (info, follower_ids) = {
      let inner = self.inner.lock().await;
      let session = inner
        .sessions
        .get(session_id)
        .ok_or_else(|| serde_json::json!({ "code": "SYNC_SESSION_NOT_FOUND" }).to_string())?;
      (
        session.info(),
        session
          .followers
          .iter()
          .map(|follower| follower.profile_id.clone())
          .collect::<Vec<_>>(),
      )
    };

    if follower_ids.is_empty() {
      return Ok(info);
    }

    let (screen_width, screen_height) = crate::wayfern_manager::host_screen_size(&app_handle)
      .ok_or_else(|| serde_json::json!({ "code": "SYNC_DISPLAY_UNAVAILABLE" }).to_string())?;

    let rects = layout_windows(layout, follower_ids.len(), screen_width, screen_height);
    if !layout_fits(&rects) {
      return Err(
        serde_json::json!({
          "code": "SYNC_DISPLAY_TOO_SMALL",
          "params": { "count": follower_ids.len().to_string() }
        })
        .to_string(),
      );
    }

    let mut placed = 0usize;
    for (follower_id, rect) in follower_ids.iter().zip(rects) {
      match Self::place_window(follower_id, rect).await {
        Ok(()) => placed += 1,
        Err(e) => log::warn!("Synchronizer: could not place follower {follower_id}: {e}"),
      }
    }

    if placed == 0 {
      return Err(serde_json::json!({ "code": "SYNC_ARRANGE_FAILED" }).to_string());
    }
    log::info!(
      "Synchronizer session {session_id}: placed {placed} of {} windows",
      follower_ids.len()
    );
    Ok(info)
  }

  /// Move one follower's browser window through CDP.
  async fn place_window(follower_profile_id: &str, rect: WindowRect) -> Result<(), String> {
    use futures_util::sink::SinkExt;
    use futures_util::stream::StreamExt;
    use tokio_tungstenite::tungstenite::Message;

    let profile = Self::get_profile(follower_profile_id)?;
    let port = Self::get_cdp_port(&profile).await?;
    let ws_url = Self::get_page_ws_url(port).await?;
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
      .await
      .map_err(|e| format!("Failed to connect to follower CDP: {e}"))?;

    let ask = serde_json::json!({ "id": 1, "method": "Browser.getWindowForTarget", "params": {} });
    ws.send(Message::Text(ask.to_string().into()))
      .await
      .map_err(|e| format!("Failed to ask for the window: {e}"))?;

    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(5);
    let window_id = loop {
      let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
      if remaining.is_zero() {
        return Err("Timed out waiting for the window id".to_string());
      }
      match tokio::time::timeout(remaining, ws.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => {
          let Ok(response) = serde_json::from_str::<serde_json::Value>(text.as_str()) else {
            continue;
          };
          if response.get("id") != Some(&serde_json::json!(1)) {
            continue;
          }
          match response
            .get("result")
            .and_then(|r| r.get("windowId"))
            .and_then(|w| w.as_i64())
          {
            Some(id) => break id,
            None => return Err("The browser reported no window".to_string()),
          }
        }
        Ok(Some(Ok(_))) => continue,
        Ok(Some(Err(e))) => return Err(format!("WebSocket error: {e}")),
        Ok(None) => return Err("WebSocket closed".to_string()),
        Err(_) => return Err("Timed out waiting for the window id".to_string()),
      }
    };

    // `normal` travels with the bounds so a maximised or minimised window is
    // restored first; without it Chromium refuses to move the window at all.
    let place = serde_json::json!({
      "id": 2,
      "method": "Browser.setWindowBounds",
      "params": {
        "windowId": window_id,
        "bounds": {
          "left": rect.left,
          "top": rect.top,
          "width": rect.width,
          "height": rect.height,
          "windowState": "normal",
        }
      }
    });
    ws.send(Message::Text(place.to_string().into()))
      .await
      .map_err(|e| format!("Failed to move the window: {e}"))?;

    // Wait for the acknowledgement rather than firing and forgetting: the
    // panel reports how many windows actually moved.
    match tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await {
      Ok(Some(Ok(Message::Text(text)))) => {
        let response: serde_json::Value = serde_json::from_str(text.as_str()).unwrap_or_default();
        if let Some(error) = response.get("error") {
          return Err(format!("CDP refused the move: {error}"));
        }
        Ok(())
      }
      Ok(Some(Ok(_))) | Ok(None) => Ok(()),
      Ok(Some(Err(e))) => Err(format!("WebSocket error: {e}")),
      Err(_) => Err("Timed out moving the window".to_string()),
    }
  }

  /// Get all active sync sessions.
  pub async fn get_sessions(&self) -> Vec<SyncSessionInfo> {
    let inner = self.inner.lock().await;
    inner.sessions.values().map(SyncSession::info).collect()
  }

  // --- Helper methods ---

  fn get_profile(profile_id: &str) -> Result<BrowserProfile, String> {
    let profiles = ProfileManager::instance()
      .list_profiles()
      .map_err(|e| format!("Failed to list profiles: {e}"))?;
    profiles
      .into_iter()
      .find(|p| p.id.to_string() == profile_id)
      .ok_or(format!("Profile '{profile_id}' not found"))
  }

  async fn get_cdp_port(profile: &BrowserProfile) -> Result<u16, String> {
    let profiles_dir = ProfileManager::instance().get_profiles_dir();
    let profile_path = profile.get_profile_data_path(&profiles_dir);
    let profile_path_str = profile_path.to_string_lossy();

    for attempt in 0..15 {
      if attempt > 0 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
      }
      let port = crate::wayfern_manager::WayfernManager::instance()
        .get_cdp_port(&profile_path_str)
        .await;
      if let Some(p) = port {
        return Ok(p);
      }
    }
    Err(format!(
      "No CDP port available for profile '{}'. Browser may not be running.",
      profile.name
    ))
  }

  /// Get a page-level WebSocket URL
  async fn get_page_ws_url(port: u16) -> Result<String, String> {
    let url = format!("http://127.0.0.1:{port}/json");
    let client = reqwest::Client::new();
    for attempt in 0..15 {
      if attempt > 0 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
      }
      if let Ok(resp) = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
      {
        if let Ok(targets) = resp.json::<Vec<serde_json::Value>>().await {
          if let Some(ws_url) = targets
            .iter()
            .find(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
            .and_then(|t| t.get("webSocketDebuggerUrl"))
            .and_then(|v| v.as_str())
          {
            return Ok(ws_url.to_string());
          }
        }
      }
    }
    Err("Failed to get CDP WebSocket URL".to_string())
  }
}

// --- Tauri Commands ---

#[tauri::command]
pub async fn start_sync_session(
  app_handle: tauri::AppHandle,
  leader_profile_id: String,
  follower_profile_ids: Vec<String>,
) -> Result<SyncSessionInfo, String> {
  SynchronizerManager::instance()
    .start_session(app_handle, leader_profile_id, follower_profile_ids)
    .await
}

#[tauri::command]
pub async fn stop_sync_session(
  app_handle: tauri::AppHandle,
  session_id: String,
) -> Result<(), String> {
  SynchronizerManager::instance()
    .stop_session(app_handle, &session_id)
    .await
}

#[tauri::command]
pub async fn remove_sync_follower(
  app_handle: tauri::AppHandle,
  session_id: String,
  follower_profile_id: String,
) -> Result<(), String> {
  SynchronizerManager::instance()
    .remove_follower(app_handle, &session_id, &follower_profile_id)
    .await
}

#[tauri::command]
pub async fn get_sync_sessions() -> Result<Vec<SyncSessionInfo>, String> {
  Ok(SynchronizerManager::instance().get_sessions().await)
}

#[tauri::command]
pub async fn set_sync_session_paused(
  app_handle: tauri::AppHandle,
  session_id: String,
  paused: bool,
) -> Result<SyncSessionInfo, String> {
  SynchronizerManager::instance()
    .set_paused(app_handle, &session_id, paused)
    .await
}

#[tauri::command]
pub async fn set_sync_follower_held(
  app_handle: tauri::AppHandle,
  session_id: String,
  follower_profile_id: String,
  held: bool,
) -> Result<SyncSessionInfo, String> {
  SynchronizerManager::instance()
    .set_follower_held(app_handle, &session_id, &follower_profile_id, held)
    .await
}

#[tauri::command]
pub async fn arrange_sync_windows(
  app_handle: tauri::AppHandle,
  session_id: String,
  layout: WindowLayout,
) -> Result<SyncSessionInfo, String> {
  SynchronizerManager::instance()
    .arrange_windows(app_handle, &session_id, layout)
    .await
}

#[cfg(test)]
mod tests {
  use super::*;

  fn overlaps(a: &WindowRect, b: &WindowRect) -> bool {
    let a_right = a.left + a.width as i32;
    let a_bottom = a.top + a.height as i32;
    let b_right = b.left + b.width as i32;
    let b_bottom = b.top + b.height as i32;
    a.left < b_right && b.left < a_right && a.top < b_bottom && b.top < a_bottom
  }

  fn on_screen(rect: &WindowRect, width: u32, height: u32) -> bool {
    rect.left >= 0
      && rect.top >= 0
      && rect.width > 0
      && rect.height > 0
      && rect.left + rect.width as i32 <= width as i32
      && rect.top + rect.height as i32 <= height as i32
  }

  #[test]
  fn every_layout_keeps_one_to_nine_windows_on_the_display() {
    for (width, height) in [(1920u32, 1080u32), (1280, 800), (2560, 1440), (1440, 900)] {
      for count in 1..=9usize {
        for layout in [
          WindowLayout::Grid,
          WindowLayout::Columns,
          WindowLayout::Cascade,
        ] {
          let rects = layout_windows(layout, count, width, height);
          assert_eq!(rects.len(), count, "{layout:?} produced the wrong count");
          for rect in &rects {
            assert!(
              on_screen(rect, width, height),
              "{layout:?} put {rect:?} outside {width}x{height} for {count} windows"
            );
          }
        }
      }
    }
  }

  #[test]
  fn the_grid_never_overlaps() {
    for (width, height) in [(1920u32, 1080u32), (1366, 768)] {
      for count in 1..=9usize {
        let rects = layout_windows(WindowLayout::Grid, count, width, height);
        for (i, a) in rects.iter().enumerate() {
          for b in rects.iter().skip(i + 1) {
            assert!(!overlaps(a, b), "{a:?} overlaps {b:?} for {count} windows");
          }
        }
      }
    }
  }

  #[test]
  fn the_grid_is_as_square_as_the_count_allows() {
    // Four windows are two by two, not four in a row: the whole point of the
    // grid rather than the columns layout.
    let rects = layout_windows(WindowLayout::Grid, 4, 1600, 1000);
    assert_eq!(
      rects[0],
      WindowRect {
        left: 0,
        top: 0,
        width: 800,
        height: 500
      }
    );
    assert_eq!(
      rects[1],
      WindowRect {
        left: 800,
        top: 0,
        width: 800,
        height: 500
      }
    );
    assert_eq!(
      rects[2],
      WindowRect {
        left: 0,
        top: 500,
        width: 800,
        height: 500
      }
    );
    assert_eq!(
      rects[3],
      WindowRect {
        left: 800,
        top: 500,
        width: 800,
        height: 500
      }
    );

    // A single window fills the display rather than sitting in a corner.
    assert_eq!(
      layout_windows(WindowLayout::Grid, 1, 1600, 1000),
      vec![WindowRect {
        left: 0,
        top: 0,
        width: 1600,
        height: 1000
      }]
    );
  }

  #[test]
  fn columns_run_left_to_right_at_full_height() {
    let rects = layout_windows(WindowLayout::Columns, 3, 1500, 900);
    for (index, rect) in rects.iter().enumerate() {
      assert_eq!(rect.top, 0);
      assert_eq!(rect.height, 900);
      assert_eq!(rect.width, 500);
      assert_eq!(rect.left, index as i32 * 500);
    }
    for (i, a) in rects.iter().enumerate() {
      for b in rects.iter().skip(i + 1) {
        assert!(!overlaps(a, b), "columns must not overlap either");
      }
    }
  }

  #[test]
  fn a_cascade_steps_down_and_right_without_leaving_the_display() {
    let rects = layout_windows(WindowLayout::Cascade, 5, 1920, 1080);
    for pair in rects.windows(2) {
      assert!(pair[1].left > pair[0].left, "each window steps right");
      assert!(pair[1].top > pair[0].top, "each window steps down");
      assert_eq!(pair[1].width, pair[0].width, "a cascade keeps one size");
    }
    let last = rects.last().unwrap();
    assert!(on_screen(last, 1920, 1080));

    // One window has nowhere to step to, so it sits at the origin.
    let single = layout_windows(WindowLayout::Cascade, 1, 1920, 1080);
    assert_eq!(single[0].left, 0);
    assert_eq!(single[0].top, 0);
  }

  #[test]
  fn nothing_is_arranged_without_windows_or_a_display() {
    assert!(layout_windows(WindowLayout::Grid, 0, 1920, 1080).is_empty());
    assert!(layout_windows(WindowLayout::Columns, 3, 0, 1080).is_empty());
    assert!(layout_windows(WindowLayout::Cascade, 3, 1920, 0).is_empty());
  }

  #[test]
  fn a_display_too_small_to_tile_is_reported_rather_than_tiled() {
    assert!(layout_fits(&layout_windows(
      WindowLayout::Grid,
      9,
      1920,
      1080
    )));
    // Nine columns on a 1280 wide display gives 142px windows, which no
    // browser will honour.
    assert!(!layout_fits(&layout_windows(
      WindowLayout::Columns,
      9,
      1280,
      800
    )));
  }

  #[test]
  fn holding_a_follower_out_stops_only_that_one() {
    let mut gate = MirrorGate::default();
    assert!(gate.accepts("a"));
    assert!(gate.accepts("b"));

    gate.set_held("a", true);
    assert!(gate.is_held("a"));
    assert!(!gate.accepts("a"));
    assert!(
      gate.accepts("b"),
      "holding one follower must not silence another"
    );

    gate.set_held("a", false);
    assert!(!gate.is_held("a"));
    assert!(gate.accepts("a"), "a follower comes back exactly as it was");

    // Setting the same state twice is not a toggle.
    gate.set_held("a", true);
    gate.set_held("a", true);
    assert!(!gate.accepts("a"));
    gate.set_held("a", false);
    assert!(gate.accepts("a"));
  }

  #[test]
  fn pausing_stops_everyone_and_resuming_restores_the_held_set() {
    let mut gate = MirrorGate::default();
    gate.set_held("held", true);

    gate.set_paused(true);
    assert!(gate.is_paused());
    assert!(!gate.accepts("free"), "a pause covers every follower");
    assert!(!gate.accepts("held"));

    gate.set_paused(false);
    assert!(!gate.is_paused());
    assert!(gate.accepts("free"));
    assert!(
      !gate.accepts("held"),
      "resuming must not quietly un-hold a follower the user held out"
    );
  }

  #[test]
  fn a_follower_that_leaves_does_not_stay_held() {
    let mut gate = MirrorGate::default();
    gate.set_held("gone", true);
    gate.forget("gone");
    assert!(!gate.is_held("gone"));
    assert!(
      gate.accepts("gone"),
      "a profile that rejoins later must not inherit the old hold"
    );
  }

  #[test]
  fn a_layout_round_trips_through_the_wire_format() {
    // The panel sends these names; a rename here has to fail loudly.
    assert_eq!(
      serde_json::from_str::<WindowLayout>("\"grid\"").unwrap(),
      WindowLayout::Grid
    );
    assert_eq!(
      serde_json::from_str::<WindowLayout>("\"columns\"").unwrap(),
      WindowLayout::Columns
    );
    assert_eq!(
      serde_json::from_str::<WindowLayout>("\"cascade\"").unwrap(),
      WindowLayout::Cascade
    );
    assert!(serde_json::from_str::<WindowLayout>("\"tiled\"").is_err());
  }

  #[test]
  fn session_info_carries_the_pause_and_hold_state() {
    let gate: SharedGate = Arc::new(std::sync::RwLock::new(MirrorGate::default()));
    let (cancel_tx, _cancel_rx) = tokio::sync::watch::channel(false);
    let session = SyncSession {
      id: "session".to_string(),
      leader_profile_id: "leader".to_string(),
      leader_profile_name: "Leader".to_string(),
      followers: vec![
        SyncFollowerState {
          profile_id: "one".to_string(),
          profile_name: "One".to_string(),
          failed_at_url: None,
          held: false,
        },
        SyncFollowerState {
          profile_id: "two".to_string(),
          profile_name: "Two".to_string(),
          failed_at_url: None,
          held: false,
        },
      ],
      gate: gate.clone(),
      cancel_tx,
    };

    let info = session.info();
    assert!(!info.paused);
    // The order the followers were chosen in is the order the panel and the
    // arrangement number them by.
    assert_eq!(
      info
        .followers
        .iter()
        .map(|f| f.profile_id.as_str())
        .collect::<Vec<_>>(),
      vec!["one", "two"]
    );

    gate.write().unwrap().set_paused(true);
    gate.write().unwrap().set_held("two", true);

    let info = session.info();
    assert!(info.paused);
    assert!(!info.followers[0].held);
    assert!(info.followers[1].held);
    assert!(session.has_follower("one"));
    assert!(!session.has_follower("missing"));
  }
}
