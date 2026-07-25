use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::cloud_auth::CLOUD_AUTH;

const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitOutcome {
  Unlimited,
  Allowed { remaining: u64 },
  Limited { retry_after_secs: u64 },
}

#[derive(Default)]
struct AutomationRateLimiter {
  requests: HashMap<String, VecDeque<Instant>>,
}

impl AutomationRateLimiter {
  fn check_at(&mut self, identity: &str, requests_per_hour: u64, now: Instant) -> RateLimitOutcome {
    if requests_per_hour == 0 {
      return RateLimitOutcome::Unlimited;
    }

    self.requests.retain(|_, requests| {
      while requests
        .front()
        .is_some_and(|started| now.duration_since(*started) >= RATE_LIMIT_WINDOW)
      {
        requests.pop_front();
      }
      !requests.is_empty()
    });

    let requests = self.requests.entry(identity.to_string()).or_default();
    if requests.len() as u64 >= requests_per_hour {
      let retry_after_secs = requests
        .front()
        .map(|started| {
          let remaining = RATE_LIMIT_WINDOW.saturating_sub(now.duration_since(*started));
          remaining
            .as_secs()
            .saturating_add(u64::from(remaining.subsec_nanos() > 0))
            .max(1)
        })
        .unwrap_or(1);
      return RateLimitOutcome::Limited { retry_after_secs };
    }

    requests.push_back(now);
    RateLimitOutcome::Allowed {
      remaining: requests_per_hour.saturating_sub(requests.len() as u64),
    }
  }
}

static AUTOMATION_RATE_LIMITER: LazyLock<Mutex<AutomationRateLimiter>> =
  LazyLock::new(|| Mutex::new(AutomationRateLimiter::default()));

pub async fn check_automation_rate_limit() -> RateLimitOutcome {
  let Some((identity, requests_per_hour)) = CLOUD_AUTH.automation_rate_limit().await else {
    return RateLimitOutcome::Unlimited;
  };

  AUTOMATION_RATE_LIMITER
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
    .check_at(&identity, requests_per_hour, Instant::now())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rolling_window_limits_per_identity_and_recovers() {
    let mut limiter = AutomationRateLimiter::default();
    let now = Instant::now();

    assert_eq!(
      limiter.check_at("user-a", 2, now),
      RateLimitOutcome::Allowed { remaining: 1 }
    );
    assert_eq!(
      limiter.check_at("user-a", 2, now + Duration::from_secs(1)),
      RateLimitOutcome::Allowed { remaining: 0 }
    );
    assert_eq!(
      limiter.check_at("user-a", 2, now + Duration::from_secs(2)),
      RateLimitOutcome::Limited {
        retry_after_secs: 3598
      }
    );

    assert_eq!(
      limiter.check_at("user-b", 2, now + Duration::from_secs(2)),
      RateLimitOutcome::Allowed { remaining: 1 }
    );
    assert_eq!(
      limiter.check_at("user-a", 2, now + RATE_LIMIT_WINDOW),
      RateLimitOutcome::Allowed { remaining: 0 }
    );
    assert_eq!(
      limiter.check_at(
        "user-a",
        2,
        now + RATE_LIMIT_WINDOW + Duration::from_secs(1)
      ),
      RateLimitOutcome::Allowed { remaining: 0 }
    );
    assert_eq!(
      limiter.check_at(
        "user-a",
        2,
        now + RATE_LIMIT_WINDOW * 2 + Duration::from_secs(1)
      ),
      RateLimitOutcome::Allowed { remaining: 1 }
    );
  }

  #[test]
  fn zero_limit_is_unlimited_and_does_not_consume_capacity() {
    let mut limiter = AutomationRateLimiter::default();
    let now = Instant::now();

    assert_eq!(
      limiter.check_at("user-a", 0, now),
      RateLimitOutcome::Unlimited
    );
    assert_eq!(
      limiter.check_at("user-a", 1, now),
      RateLimitOutcome::Allowed { remaining: 0 }
    );
  }
}
