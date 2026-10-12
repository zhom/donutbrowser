//! When the wait after an action that may navigate can stop.
//!
//! The action's reply alone does not end the wait: a link click is answered
//! before the page it opens has loaded. But most clicks open a menu or change
//! a field and never load anything, and waiting the full load timeout after
//! each of them cost an agent seconds per click. So the wait ends:
//!
//! - when the main frame fires its load event, or navigates within the
//!   document (a single-page app changing its route); or
//! - a short grace after the reply, when the main frame has not started to
//!   navigate by then.
//!
//! A frame other than the main one (an ad, a widget) loading does not count:
//! its load is not the page the agent acted on.

use serde_json::Value;
use std::time::Duration;
use tokio::time::Instant;

/// How long after the reply a navigation may still start. A click handler
/// that navigates usually does so at once; some wait a moment (an animation,
/// a request) first.
pub const NAVIGATION_GRACE: Duration = Duration::from_millis(500);

/// The CDP events that say the main frame is on its way to a new document.
const STARTED: [&str; 3] = [
  "Page.frameRequestedNavigation",
  "Page.frameStartedLoading",
  "Page.frameScheduledNavigation",
];

#[derive(Debug)]
pub struct NavigationWatch {
  /// The page's top frame. `None` when it could not be read: then any frame
  /// starting to load counts, which only ever waits longer.
  main_frame: Option<String>,
  /// The action is a navigation (`Page.navigate`): always wait for the load.
  expected: bool,
  grace: Duration,
  replied_at: Option<Instant>,
  started: bool,
  loaded: bool,
  same_document: bool,
}

impl NavigationWatch {
  pub fn new(main_frame: Option<String>, expected: bool) -> Self {
    Self {
      main_frame,
      expected,
      grace: NAVIGATION_GRACE,
      replied_at: None,
      started: false,
      loaded: false,
      same_document: false,
    }
  }

  pub fn on_reply(&mut self, now: Instant) {
    self.replied_at.get_or_insert(now);
  }

  pub fn on_event(&mut self, method: &str, params: Option<&Value>) {
    let in_main_frame = || match (&self.main_frame, params.and_then(|p| p.get("frameId"))) {
      (Some(main), Some(frame)) => frame.as_str() == Some(main.as_str()),
      _ => true,
    };
    if method == "Page.loadEventFired" {
      self.loaded = true;
    } else if method == "Page.frameNavigated" && restored_from_cache(params) {
      // A page the back-forward cache keeps comes back whole: no load event.
      self.loaded = true;
    } else if method == "Page.navigatedWithinDocument" && in_main_frame() {
      self.same_document = true;
    } else if STARTED.contains(&method) && in_main_frame() {
      self.started = true;
    }
  }

  /// Whether the action took the page somewhere: a load, or a route change.
  pub fn navigated(&self) -> bool {
    self.loaded || self.same_document
  }

  pub fn done(&self, now: Instant) -> bool {
    let Some(replied_at) = self.replied_at else {
      return false;
    };
    if self.navigated() {
      return true;
    }
    !self.expected && !self.started && now >= replied_at + self.grace
  }

  /// When to look again: the deadline, or sooner when the grace ends first.
  pub fn wake_at(&self, deadline: Instant) -> Instant {
    match self.replied_at {
      Some(replied_at) if !self.expected && !self.started => deadline.min(replied_at + self.grace),
      _ => deadline,
    }
  }
}

/// Whether a `Page.frameNavigated` brought back the top frame from the
/// back-forward cache.
fn restored_from_cache(params: Option<&Value>) -> bool {
  params.is_some_and(|p| {
    p.get("type").and_then(Value::as_str) == Some("BackForwardCacheRestore")
      && p.pointer("/frame/parentId").is_none()
  })
}

/// The top frame's id, from a `Page.getFrameTree` reply.
pub fn main_frame_of(tree: &Value) -> Option<String> {
  tree
    .pointer("/frameTree/frame/id")
    .and_then(Value::as_str)
    .map(str::to_string)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn watch() -> NavigationWatch {
    NavigationWatch::new(Some("main".to_string()), false)
  }

  #[test]
  fn a_click_that_loads_nothing_ends_after_the_grace() {
    let start = Instant::now();
    let mut watch = watch();
    assert!(!watch.done(start), "never before the reply");
    watch.on_reply(start);
    assert!(!watch.done(start + Duration::from_millis(100)));
    assert_eq!(
      watch.wake_at(start + Duration::from_secs(10)),
      start + NAVIGATION_GRACE
    );
    assert!(watch.done(start + NAVIGATION_GRACE));
    assert!(!watch.navigated());
  }

  #[test]
  fn a_page_back_from_the_cache_ends_the_wait_without_a_load_event() {
    let start = Instant::now();
    let mut watch = NavigationWatch::new(Some("main".to_string()), true);
    watch.on_reply(start);
    watch.on_event(
      "Page.frameNavigated",
      Some(
        &json!({ "frame": { "id": "ad", "parentId": "main" }, "type": "BackForwardCacheRestore" }),
      ),
    );
    assert!(!watch.done(start), "a child frame is not the page");
    watch.on_event(
      "Page.frameNavigated",
      Some(&json!({ "frame": { "id": "main" }, "type": "Navigation" })),
    );
    assert!(
      !watch.done(start),
      "a normal navigation still waits for its load"
    );
    watch.on_event(
      "Page.frameNavigated",
      Some(&json!({ "frame": { "id": "main" }, "type": "BackForwardCacheRestore" })),
    );
    assert!(watch.done(start));
    assert!(watch.navigated());
  }

  #[test]
  fn a_started_navigation_waits_for_its_load() {
    let start = Instant::now();
    let mut watch = watch();
    watch.on_reply(start);
    watch.on_event(
      "Page.frameStartedLoading",
      Some(&json!({ "frameId": "main" })),
    );
    assert!(!watch.done(start + Duration::from_secs(5)));
    assert_eq!(
      watch.wake_at(start + Duration::from_secs(10)),
      start + Duration::from_secs(10)
    );
    watch.on_event("Page.loadEventFired", Some(&json!({ "timestamp": 1.0 })));
    assert!(watch.done(start + Duration::from_secs(5)));
    assert!(watch.navigated());
  }

  #[test]
  fn another_frame_loading_does_not_hold_the_click() {
    let start = Instant::now();
    let mut watch = watch();
    watch.on_reply(start);
    watch.on_event(
      "Page.frameStartedLoading",
      Some(&json!({ "frameId": "ad" })),
    );
    assert!(watch.done(start + NAVIGATION_GRACE));
  }

  #[test]
  fn a_route_change_is_a_navigation_with_no_load() {
    let start = Instant::now();
    let mut watch = watch();
    watch.on_reply(start);
    watch.on_event(
      "Page.navigatedWithinDocument",
      Some(&json!({ "frameId": "main" })),
    );
    assert!(watch.done(start));
    assert!(watch.navigated());
  }

  #[test]
  fn an_expected_navigation_always_waits_for_the_load() {
    let start = Instant::now();
    let mut watch = NavigationWatch::new(None, true);
    watch.on_reply(start);
    assert!(!watch.done(start + Duration::from_secs(9)));
    assert_eq!(
      main_frame_of(&json!({ "frameTree": { "frame": { "id": "F1" } } })),
      Some("F1".to_string())
    );
  }
}
