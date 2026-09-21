use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::cloud_auth::CLOUD_AUTH;

const GENERATION_WINDOW: Duration = Duration::from_secs(60 * 60);
const PRO_GENERATIONS_PER_HOUR: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationOutcome {
  Unlimited,
  Allowed { remaining: u64 },
  Limited { retry_after_secs: u64 },
}

#[derive(Default)]
struct ProfileGenerationLimiter {
  generations: HashMap<String, VecDeque<Instant>>,
}

impl ProfileGenerationLimiter {
  fn record_at(&mut self, identity: &str, per_hour: u64, now: Instant) -> GenerationOutcome {
    if per_hour == 0 {
      return GenerationOutcome::Unlimited;
    }

    self.generations.retain(|_, seen| {
      while seen
        .front()
        .is_some_and(|started| now.duration_since(*started) >= GENERATION_WINDOW)
      {
        seen.pop_front();
      }
      !seen.is_empty()
    });

    let seen = self.generations.entry(identity.to_string()).or_default();
    if seen.len() as u64 >= per_hour {
      let retry_after_secs = seen
        .front()
        .map(|started| {
          let remaining = GENERATION_WINDOW.saturating_sub(now.duration_since(*started));
          remaining
            .as_secs()
            .saturating_add(u64::from(remaining.subsec_nanos() > 0))
            .max(1)
        })
        .unwrap_or(1);
      return GenerationOutcome::Limited { retry_after_secs };
    }

    seen.push_back(now);
    GenerationOutcome::Allowed {
      remaining: per_hour.saturating_sub(seen.len() as u64),
    }
  }
}

static PROFILE_GENERATION_LIMITER: LazyLock<Mutex<ProfileGenerationLimiter>> =
  LazyLock::new(|| Mutex::new(ProfileGenerationLimiter::default()));

/// Identity and cap for the accounts this limiter applies to. Only `pro` is
/// capped; every other plan returns `None` and stays unlimited here.
async fn capped_identity() -> Option<(String, u64)> {
  let state = CLOUD_AUTH.get_user().await?;
  if !state.user.entitlements().active || state.user.effective_plan() != "pro" {
    return None;
  }
  Some((state.user.id.clone(), PRO_GENERATIONS_PER_HOUR))
}

pub async fn record_profile_generation() -> GenerationOutcome {
  let Some((identity, per_hour)) = capped_identity().await else {
    return GenerationOutcome::Unlimited;
  };

  PROFILE_GENERATION_LIMITER
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
    .record_at(&identity, per_hour, Instant::now())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_hundred_and_first_generation_in_an_hour_is_refused() {
    let mut limiter = ProfileGenerationLimiter::default();
    let now = Instant::now();

    for i in 0..PRO_GENERATIONS_PER_HOUR {
      assert_eq!(
        limiter.record_at(
          "user-a",
          PRO_GENERATIONS_PER_HOUR,
          now + Duration::from_secs(i)
        ),
        GenerationOutcome::Allowed {
          remaining: PRO_GENERATIONS_PER_HOUR - i - 1
        }
      );
    }

    assert_eq!(
      limiter.record_at(
        "user-a",
        PRO_GENERATIONS_PER_HOUR,
        now + Duration::from_secs(PRO_GENERATIONS_PER_HOUR)
      ),
      GenerationOutcome::Limited {
        retry_after_secs: 3600 - PRO_GENERATIONS_PER_HOUR
      }
    );
  }

  #[test]
  fn the_window_rolls_so_the_oldest_slot_comes_back() {
    let mut limiter = ProfileGenerationLimiter::default();
    let now = Instant::now();

    assert_eq!(
      limiter.record_at("user-a", 2, now),
      GenerationOutcome::Allowed { remaining: 1 }
    );
    assert_eq!(
      limiter.record_at("user-a", 2, now + Duration::from_secs(10)),
      GenerationOutcome::Allowed { remaining: 0 }
    );
    assert_eq!(
      limiter.record_at("user-a", 2, now + Duration::from_secs(20)),
      GenerationOutcome::Limited {
        retry_after_secs: 3580
      }
    );
    assert_eq!(
      limiter.record_at("user-a", 2, now + GENERATION_WINDOW),
      GenerationOutcome::Allowed { remaining: 0 }
    );
  }

  #[test]
  fn each_account_gets_its_own_window() {
    let mut limiter = ProfileGenerationLimiter::default();
    let now = Instant::now();

    assert_eq!(
      limiter.record_at("user-a", 1, now),
      GenerationOutcome::Allowed { remaining: 0 }
    );
    assert!(matches!(
      limiter.record_at("user-a", 1, now + Duration::from_secs(1)),
      GenerationOutcome::Limited { .. }
    ));
    assert_eq!(
      limiter.record_at("user-b", 1, now + Duration::from_secs(1)),
      GenerationOutcome::Allowed { remaining: 0 }
    );
  }

  #[test]
  fn a_zero_cap_never_limits() {
    let mut limiter = ProfileGenerationLimiter::default();
    let now = Instant::now();

    for i in 0..500 {
      assert_eq!(
        limiter.record_at("user-a", 0, now + Duration::from_secs(i)),
        GenerationOutcome::Unlimited
      );
    }
  }
}
