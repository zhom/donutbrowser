//! Lines that would otherwise repeat.
//!
//! `Streak`: a failure that repeats, logged as the first failure, a reminder at
//! most once an hour while it lasts, and the recovery with its count. A backend
//! that is unreachable for a day writes about 25 lines, not thousands. A
//! failure within a minute of a recovery continues the old streak as a flap,
//! so a link that connects and drops again writes a few lines an hour, not a
//! pair per cycle.
//! `KeyedStreak`: the same, one streak per entity, so one entity's success
//! never hides another's failure. `Throttle`: an event that is normal one at a
//! time but noisy in bulk (a refused request), written at most once per window
//! per key with the count it dropped.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const REMINDER_EVERY: Duration = Duration::from_secs(3600);
const SETTLE: Duration = Duration::from_secs(60);

/// Declare as a `static` in the module that fails, so lines keep its target:
/// `static SSE: Streak = Streak::new(module_path!(), "Sync SSE connect");`
pub struct Streak {
  target: &'static str,
  what: &'static str,
  state: Mutex<Option<State>>,
}

/// Like `Streak`, with one streak per key such as `profile=<uuid>`. A key is
/// dropped once it has stayed recovered past the flap window.
pub struct KeyedStreak {
  target: &'static str,
  what: &'static str,
  states: Mutex<BTreeMap<String, State>>,
}

/// `if let Some(dropped) = REFUSED.allow(&key) { log::warn!("...{}", suppressed(dropped)) }`
pub struct Throttle {
  window: Duration,
  slots: Mutex<BTreeMap<String, (Instant, u64)>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct State {
  failures: u64,
  since: Instant,
  reminded: Instant,
  /// Set while the last word was a success.
  recovered_at: Option<Instant>,
  flaps: u64,
  recovery_logged: Instant,
}

impl State {
  fn settled(&self, now: Instant) -> bool {
    self
      .recovered_at
      .is_some_and(|at| now.duration_since(at) >= SETTLE)
  }
}

#[derive(Debug, PartialEq, Eq)]
enum Event {
  Started,
  StillFailing {
    failures: u64,
    for_s: u64,
  },
  Recovered {
    failures: u64,
    after_s: u64,
    flaps: u64,
  },
}

impl Streak {
  pub const fn new(target: &'static str, what: &'static str) -> Self {
    Streak {
      target,
      what,
      state: Mutex::new(None),
    }
  }

  pub fn failed(&self, err: impl Display) {
    let event = match self.state.lock() {
      Ok(mut state) => on_failure(&mut state, Instant::now()),
      Err(_) => Some(Event::Started),
    };
    emit(self.target, self.what, "", event, Some(&err));
  }

  pub fn succeeded(&self) {
    let event = match self.state.lock() {
      Ok(mut state) => on_success(&mut state, Instant::now()),
      Err(_) => None,
    };
    emit(self.target, self.what, "", event, None);
  }
}

impl KeyedStreak {
  pub const fn new(target: &'static str, what: &'static str) -> Self {
    KeyedStreak {
      target,
      what,
      states: Mutex::new(BTreeMap::new()),
    }
  }

  pub fn failed(&self, key: &str, err: impl Display) {
    let event = match self.states.lock() {
      Ok(mut states) => {
        let now = Instant::now();
        if states.len() > 256 {
          states.retain(|_, state| !state.settled(now));
        }
        let mut state = states.get(key).copied();
        let event = on_failure(&mut state, now);
        if let Some(state) = state {
          states.insert(key.to_string(), state);
        }
        event
      }
      Err(_) => Some(Event::Started),
    };
    emit(self.target, self.what, key, event, Some(&err));
  }

  pub fn succeeded(&self, key: &str) {
    let event = match self.states.lock() {
      Ok(mut states) => {
        let mut state = states.get(key).copied();
        let event = on_success(&mut state, Instant::now());
        if let Some(state) = state {
          states.insert(key.to_string(), state);
        }
        event
      }
      Err(_) => None,
    };
    emit(self.target, self.what, key, event, None);
  }
}

impl Throttle {
  pub const fn new(window: Duration) -> Self {
    Throttle {
      window,
      slots: Mutex::new(BTreeMap::new()),
    }
  }

  /// `Some(dropped)` when a line for `key` should be written now, with the
  /// count skipped since the last one; `None` to skip it.
  pub fn allow(&self, key: &str) -> Option<u64> {
    let Ok(mut slots) = self.slots.lock() else {
      return Some(0);
    };
    let now = Instant::now();
    if slots.len() > 1024 {
      let window = self.window;
      slots.retain(|_, (last, _)| now.duration_since(*last) < window * 2);
    }
    let mut slot = slots.get(key).copied();
    let allowed = throttle_step(&mut slot, now, self.window);
    if let Some(slot) = slot {
      slots.insert(key.to_string(), slot);
    }
    allowed
  }
}

/// `" suppressed=N"` for a throttled line that dropped N others, else `""`.
pub fn suppressed(dropped: u64) -> String {
  if dropped == 0 {
    String::new()
  } else {
    format!(" suppressed={dropped}")
  }
}

fn emit(target: &str, what: &str, key: &str, event: Option<Event>, err: Option<&dyn Display>) {
  let key = if key.is_empty() {
    String::new()
  } else {
    format!(" {key}")
  };
  let err = err.map(|err| format!(" err=\"{err}\"")).unwrap_or_default();
  match event {
    Some(Event::Started) => log::warn!(target: target, "{what} failed{key}{err}"),
    Some(Event::StillFailing { failures, for_s }) => log::warn!(
      target: target,
      "{what} still failing{key} failures={failures} for_s={for_s}{err}"
    ),
    Some(Event::Recovered {
      failures,
      after_s,
      flaps,
    }) => {
      let flaps = if flaps == 0 {
        String::new()
      } else {
        format!(" flaps={flaps}")
      };
      log::info!(
        target: target,
        "{what} recovered{key} failures={failures} after_s={after_s}{flaps}"
      )
    }
    None => {}
  }
}

fn on_failure(state: &mut Option<State>, now: Instant) -> Option<Event> {
  let streak = match state {
    Some(streak) if !streak.settled(now) => streak,
    _ => {
      *state = Some(State {
        failures: 1,
        since: now,
        reminded: now,
        recovered_at: None,
        flaps: 0,
        recovery_logged: now,
      });
      return Some(Event::Started);
    }
  };
  if streak.recovered_at.take().is_some() {
    streak.flaps += 1;
  }
  streak.failures += 1;
  if now.duration_since(streak.reminded) < REMINDER_EVERY {
    return None;
  }
  streak.reminded = now;
  Some(Event::StillFailing {
    failures: streak.failures,
    for_s: now.duration_since(streak.since).as_secs(),
  })
}

fn on_success(state: &mut Option<State>, now: Instant) -> Option<Event> {
  let streak = state.as_mut()?;
  if streak.recovered_at.is_some() {
    return None;
  }
  streak.recovered_at = Some(now);
  if streak.flaps > 0 && now.duration_since(streak.recovery_logged) < REMINDER_EVERY {
    return None;
  }
  streak.recovery_logged = now;
  Some(Event::Recovered {
    failures: streak.failures,
    after_s: now.duration_since(streak.since).as_secs(),
    flaps: streak.flaps,
  })
}

fn throttle_step(slot: &mut Option<(Instant, u64)>, now: Instant, window: Duration) -> Option<u64> {
  match slot {
    None => {
      *slot = Some((now, 0));
      Some(0)
    }
    Some((last, dropped)) if now.duration_since(*last) >= window => {
      let skipped = *dropped;
      *last = now;
      *dropped = 0;
      Some(skipped)
    }
    Some((_, dropped)) => {
      *dropped += 1;
      None
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_streak_logs_its_start_an_hourly_reminder_and_its_end() {
    let start = Instant::now();
    let mut state = None;

    assert_eq!(on_failure(&mut state, start), Some(Event::Started));
    for minute in 1..60 {
      let now = start + Duration::from_secs(minute * 60);
      assert_eq!(on_failure(&mut state, now), None, "minute {minute}");
    }
    assert_eq!(
      on_failure(&mut state, start + REMINDER_EVERY),
      Some(Event::StillFailing {
        failures: 61,
        for_s: 3600
      })
    );
    assert_eq!(
      on_success(&mut state, start + Duration::from_secs(3700)),
      Some(Event::Recovered {
        failures: 61,
        after_s: 3700,
        flaps: 0
      })
    );
    assert_eq!(
      on_success(&mut state, start + Duration::from_secs(3800)),
      None,
      "a second success says nothing"
    );
  }

  #[test]
  fn success_without_a_streak_says_nothing() {
    let mut state = None;
    assert_eq!(on_success(&mut state, Instant::now()), None);
  }

  #[test]
  fn a_failure_after_a_settled_recovery_starts_a_new_streak() {
    let start = Instant::now();
    let mut state = None;
    on_failure(&mut state, start);
    on_success(&mut state, start);
    assert_eq!(on_failure(&mut state, start + SETTLE), Some(Event::Started));
  }

  #[test]
  fn a_link_that_keeps_dropping_flaps_inside_one_streak() {
    let start = Instant::now();
    let mut state = None;
    let mut lines = Vec::new();
    // Connects for 5s, drops, reconnects 5s later, for two hours.
    for cycle in 0..720u64 {
      let failed_at = start + Duration::from_secs(cycle * 10);
      lines.extend(on_failure(&mut state, failed_at));
      lines.extend(on_success(&mut state, failed_at + Duration::from_secs(5)));
    }
    assert_eq!(lines[0], Event::Started);
    assert!(matches!(lines[1], Event::Recovered { flaps: 0, .. }));
    assert!(
      lines.len() <= 6,
      "two hours of flapping wrote {} lines: {lines:?}",
      lines.len()
    );
    assert!(lines
      .iter()
      .any(|line| matches!(line, Event::Recovered { flaps, .. } if *flaps > 0)));
  }

  #[test]
  fn one_key_recovering_leaves_another_key_failing() {
    let streak = KeyedStreak::new(module_path!(), "Report");
    streak.failed("profile=a", "down");
    streak.failed("profile=b", "down");
    streak.succeeded("profile=a");

    let states = streak.states.lock().unwrap();
    assert!(states["profile=a"].recovered_at.is_some());
    assert!(states["profile=b"].recovered_at.is_none());
    assert_eq!(states["profile=b"].failures, 1);
  }

  #[test]
  fn a_throttled_key_writes_once_per_window_with_what_it_dropped() {
    let start = Instant::now();
    let window = Duration::from_secs(60);
    let mut slot = None;

    assert_eq!(throttle_step(&mut slot, start, window), Some(0));
    for second in 1..60 {
      let now = start + Duration::from_secs(second);
      assert_eq!(throttle_step(&mut slot, now, window), None);
    }
    assert_eq!(throttle_step(&mut slot, start + window, window), Some(59));
    assert_eq!(suppressed(59), " suppressed=59");
    assert_eq!(suppressed(0), "");
  }

  #[test]
  fn throttle_keys_are_independent() {
    let throttle = Throttle::new(Duration::from_secs(60));
    assert_eq!(throttle.allow("GET /a 401"), Some(0));
    assert_eq!(throttle.allow("GET /a 401"), None);
    assert_eq!(throttle.allow("GET /b 401"), Some(0));
  }
}
