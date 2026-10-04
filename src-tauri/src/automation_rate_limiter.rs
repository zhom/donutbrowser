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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct AutomationQuota {
  /// 0 means unlimited.
  pub limit: u64,
  pub used: u64,
  pub resets_in_secs: Option<u64>,
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

impl AutomationRateLimiter {
  fn usage_at(&self, identity: &str, now: Instant) -> (u64, Option<u64>) {
    let Some(requests) = self.requests.get(identity) else {
      return (0, None);
    };
    let mut live = requests
      .iter()
      .filter(|started| now.duration_since(**started) < RATE_LIMIT_WINDOW);
    let resets_in_secs = live.clone().next().map(|started| {
      RATE_LIMIT_WINDOW
        .saturating_sub(now.duration_since(*started))
        .as_secs()
        .max(1)
    });
    let used = live.by_ref().count() as u64;
    (used, resets_in_secs)
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

pub async fn automation_quota() -> Option<AutomationQuota> {
  let (identity, limit) = CLOUD_AUTH.automation_rate_limit().await?;
  let (used, resets_in_secs) = AUTOMATION_RATE_LIMITER
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
    .usage_at(&identity, Instant::now());
  Some(AutomationQuota {
    limit,
    used,
    resets_in_secs,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn usage_counts_only_the_live_window_without_consuming() {
    let mut limiter = AutomationRateLimiter::default();
    let start = Instant::now();
    assert_eq!(limiter.usage_at("a", start), (0, None));
    limiter.check_at("a", 5, start);
    limiter.check_at("a", 5, start + Duration::from_secs(10));
    let (used, resets) = limiter.usage_at("a", start + Duration::from_secs(20));
    assert_eq!(used, 2);
    assert_eq!(resets, Some(RATE_LIMIT_WINDOW.as_secs() - 20));
    assert_eq!(limiter.usage_at("a", start + Duration::from_secs(20)).0, 2);
    assert_eq!(
      limiter.usage_at("a", start + RATE_LIMIT_WINDOW + Duration::from_secs(11)),
      (0, None)
    );
  }

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
