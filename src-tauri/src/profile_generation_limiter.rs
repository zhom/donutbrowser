//! How many profiles with a freshly generated fingerprint an account creates
//! per hour.
//!
//! Paid plans are capped and a creation past the cap is refused: Solo at 10 an
//! hour, Pro, Team and every other paid plan at 100. The free allowance shrinks
//! as the profile count grows and only warns; going past it is what gets
//! generation blocked upstream, so the desktop says so before that happens.
//!
//! The window is kept on disk, so relaunching the app does not reset it.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use crate::cloud_auth::CLOUD_AUTH;

const WINDOW_SECS: u64 = 60 * 60;
const PRO_PER_HOUR: u64 = 100;
const SOLO_PER_HOUR: u64 = 10;
/// The free allowance per hour while the profile count is below each bound.
const FREE_TIERS: [(usize, u64); 3] = [(40, 8), (80, 6), (120, 4)];
const FREE_FLOOR_PER_HOUR: u64 = 2;
/// The identity generations are counted under while nobody is signed in.
const SIGNED_OUT: &str = "local";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Enforcement {
  /// Past the cap, creation is refused.
  Hard,
  /// Past the allowance, the user is warned.
  Soft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Allowance {
  pub enforcement: Enforcement,
  pub per_hour: u64,
  /// Generations in the last hour.
  pub used: u64,
  /// Seconds until enough of them leave the window to allow one more, once
  /// `used` has reached `per_hour`.
  pub retry_after_secs: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationOutcome {
  Allowed,
  Limited {
    per_hour: u64,
    retry_after_secs: u64,
  },
}

fn free_per_hour(profiles: usize) -> u64 {
  FREE_TIERS
    .iter()
    .find(|(below, _)| profiles < *below)
    .map_or(FREE_FLOOR_PER_HOUR, |(_, per_hour)| *per_hour)
}

#[derive(Default, Serialize, Deserialize)]
struct GenerationLog {
  /// Unix seconds of each generation in the last hour, per identity.
  generations: HashMap<String, Vec<u64>>,
}

impl GenerationLog {
  fn allowance(
    &mut self,
    identity: &str,
    enforcement: Enforcement,
    per_hour: u64,
    now: u64,
  ) -> Allowance {
    self.generations.retain(|_, seen| {
      seen.retain(|at| now.saturating_sub(*at) < WINDOW_SECS);
      !seen.is_empty()
    });
    let mut seen = self.generations.get(identity).cloned().unwrap_or_default();
    seen.sort_unstable();
    let used = seen.len() as u64;
    // Past a soft allowance, or after a move to a smaller plan, more than one
    // generation has to leave before the next creation fits.
    let retry_after_secs = (used >= per_hour).then(|| {
      let freeing = seen[(used - per_hour) as usize];
      (freeing + WINDOW_SECS).saturating_sub(now).max(1)
    });
    Allowance {
      enforcement,
      per_hour,
      used,
      retry_after_secs,
    }
  }

  fn record(&mut self, identity: &str, now: u64) {
    self
      .generations
      .entry(identity.to_string())
      .or_default()
      .push(now);
  }
}

struct Policy {
  identity: String,
  enforcement: Enforcement,
  per_hour: u64,
}

async fn policy() -> Policy {
  match CLOUD_AUTH.get_user().await {
    // A team member's effective plan is the owner's, so Team is capped as Pro.
    Some(state) if state.user.entitlements().active => Policy {
      enforcement: Enforcement::Hard,
      per_hour: if state.user.effective_plan() == "solo" {
        SOLO_PER_HOUR
      } else {
        PRO_PER_HOUR
      },
      identity: state.user.id,
    },
    signed_in => Policy {
      identity: signed_in.map_or_else(|| SIGNED_OUT.to_string(), |state| state.user.id),
      enforcement: Enforcement::Soft,
      per_hour: free_per_hour(
        crate::profile::ProfileManager::instance()
          .list_profiles()
          .map_or(0, |profiles| profiles.len()),
      ),
    },
  }
}

fn log_path() -> PathBuf {
  crate::app_dirs::settings_dir().join("profile_generations.json")
}

fn load() -> GenerationLog {
  std::fs::read(log_path())
    .ok()
    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    .unwrap_or_default()
}

fn save(log: &GenerationLog) {
  let path = log_path();
  let tmp = path.with_extension("json.tmp");
  let written = serde_json::to_vec(log)
    .map_err(std::io::Error::other)
    .and_then(|bytes| {
      if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
      }
      std::fs::write(&tmp, bytes)?;
      std::fs::rename(&tmp, &path)
    });
  if let Err(e) = written {
    log::warn!("Could not save the profile generation window: {e}");
  }
}

static GENERATION_LOG: LazyLock<Mutex<GenerationLog>> = LazyLock::new(|| Mutex::new(load()));

pub async fn record_profile_generation() -> GenerationOutcome {
  let policy = policy().await;
  let now = crate::proxy_manager::now_secs();
  let mut log = GENERATION_LOG
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner());
  let allowance = log.allowance(&policy.identity, policy.enforcement, policy.per_hour, now);
  if let (Enforcement::Hard, Some(retry_after_secs)) =
    (policy.enforcement, allowance.retry_after_secs)
  {
    return GenerationOutcome::Limited {
      per_hour: policy.per_hour,
      retry_after_secs,
    };
  }
  log.record(&policy.identity, now);
  save(&log);
  GenerationOutcome::Allowed
}

#[tauri::command]
pub async fn get_profile_creation_allowance() -> Allowance {
  let policy = policy().await;
  GENERATION_LOG
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
    .allowance(
      &policy.identity,
      policy.enforcement,
      policy.per_hour,
      crate::proxy_manager::now_secs(),
    )
}

#[cfg(test)]
mod tests {
  use super::*;

  const NOW: u64 = 1_800_000_000;

  #[test]
  fn the_hundred_and_first_generation_in_an_hour_is_refused() {
    let mut log = GenerationLog::default();
    for i in 0..PRO_PER_HOUR {
      let allowance = log.allowance("user-a", Enforcement::Hard, PRO_PER_HOUR, NOW + i);
      assert_eq!(allowance.used, i);
      assert_eq!(allowance.retry_after_secs, None);
      log.record("user-a", NOW + i);
    }

    let full = log.allowance(
      "user-a",
      Enforcement::Hard,
      PRO_PER_HOUR,
      NOW + PRO_PER_HOUR,
    );
    assert_eq!(full.used, PRO_PER_HOUR);
    assert_eq!(full.retry_after_secs, Some(WINDOW_SECS - PRO_PER_HOUR));
  }

  #[test]
  fn the_window_rolls_so_the_oldest_slot_comes_back() {
    let mut log = GenerationLog::default();
    log.record("user-a", NOW);
    log.record("user-a", NOW + 10);
    assert_eq!(
      log
        .allowance("user-a", Enforcement::Hard, 2, NOW + 20)
        .retry_after_secs,
      Some(3580)
    );
    let rolled = log.allowance("user-a", Enforcement::Hard, 2, NOW + WINDOW_SECS);
    assert_eq!(rolled.used, 1);
    assert_eq!(rolled.retry_after_secs, None);
  }

  #[test]
  fn past_the_allowance_the_wait_covers_every_generation_over_it() {
    let mut log = GenerationLog::default();
    for i in 0..10 {
      log.record("local", NOW + i * 60);
    }
    // Ten in the window against an allowance of eight: the third oldest has to
    // leave before an eleventh fits under it.
    let allowance = log.allowance("local", Enforcement::Soft, 8, NOW + 600);
    assert_eq!(allowance.used, 10);
    assert_eq!(allowance.retry_after_secs, Some(WINDOW_SECS - 480));
  }

  #[test]
  fn each_account_gets_its_own_window() {
    let mut log = GenerationLog::default();
    log.record("user-a", NOW);
    assert!(log
      .allowance("user-a", Enforcement::Hard, 1, NOW + 1)
      .retry_after_secs
      .is_some());
    assert_eq!(
      log
        .allowance("user-b", Enforcement::Hard, 1, NOW + 1)
        .retry_after_secs,
      None
    );
  }

  #[test]
  fn the_free_allowance_shrinks_as_the_profile_count_grows() {
    for (profiles, per_hour) in [
      (0, 8),
      (39, 8),
      (40, 6),
      (79, 6),
      (80, 4),
      (119, 4),
      (120, 2),
      (5_000, 2),
    ] {
      assert_eq!(free_per_hour(profiles), per_hour, "{profiles} profiles");
    }
  }

  #[test]
  fn the_window_survives_a_round_trip_through_its_file_format() {
    let mut log = GenerationLog::default();
    log.record("user-a", NOW);
    let mut restored: GenerationLog =
      serde_json::from_slice(&serde_json::to_vec(&log).unwrap()).unwrap();
    assert_eq!(
      restored
        .allowance("user-a", Enforcement::Hard, 1, NOW + 1)
        .used,
      1
    );
  }
}
