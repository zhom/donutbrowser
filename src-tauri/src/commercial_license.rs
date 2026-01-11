use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::AppHandle;

use crate::events;
use crate::settings_manager::SettingsManager;

const TRIAL_DURATION_SECONDS: u64 = 14 * 24 * 60 * 60; // 2 weeks

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TrialStatus {
  Active {
    remaining_seconds: u64,
    days_remaining: u64,
    hours_remaining: u64,
    minutes_remaining: u64,
  },
  Expired,
}

pub struct CommercialLicenseManager;

impl CommercialLicenseManager {
  pub fn instance() -> &'static CommercialLicenseManager {
    &COMMERCIAL_LICENSE_MANAGER
  }

  fn get_current_timestamp() -> u64 {
    SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .expect("System time before UNIX epoch")
      .as_secs()
  }

  pub async fn get_trial_status(&self, app_handle: &AppHandle) -> Result<TrialStatus, String> {
    let first_launch = self.get_or_set_first_launch(app_handle).await?;
    let now = Self::get_current_timestamp();

    if now < first_launch {
      // Clock was set back, treat as expired
      return Ok(TrialStatus::Expired);
    }

    let elapsed = now - first_launch;

    if elapsed >= TRIAL_DURATION_SECONDS {
      Ok(TrialStatus::Expired)
    } else {
      let remaining = TRIAL_DURATION_SECONDS - elapsed;
      let days = remaining / (24 * 60 * 60);
      let hours = (remaining % (24 * 60 * 60)) / (60 * 60);
      let minutes = (remaining % (60 * 60)) / 60;

      Ok(TrialStatus::Active {
        remaining_seconds: remaining,
        days_remaining: days,
        hours_remaining: hours,
        minutes_remaining: minutes,
      })
    }
  }

  async fn get_or_set_first_launch(&self, _app_handle: &AppHandle) -> Result<u64, String> {
    let settings_manager = SettingsManager::instance();
    let mut settings = settings_manager
      .load_settings()
      .map_err(|e| format!("Failed to load settings: {e}"))?;

    if let Some(timestamp) = settings.first_launch_timestamp {
      return Ok(timestamp);
    }

    // First launch - record the timestamp
    let now = Self::get_current_timestamp();
    settings.first_launch_timestamp = Some(now);
    settings_manager
      .save_settings(&settings)
      .map_err(|e| format!("Failed to save settings: {e}"))?;

    log::info!("First launch timestamp recorded: {now}");

    // Emit event to notify frontend
    if let Err(e) = events::emit("first-launch-recorded", now) {
      log::warn!("Failed to emit first-launch-recorded event: {e}");
    }

    Ok(now)
  }

  pub async fn acknowledge_expiration(&self, _app_handle: &AppHandle) -> Result<(), String> {
    let settings_manager = SettingsManager::instance();
    let mut settings = settings_manager
      .load_settings()
      .map_err(|e| format!("Failed to load settings: {e}"))?;

    settings.commercial_trial_acknowledged = true;
    settings_manager
      .save_settings(&settings)
      .map_err(|e| format!("Failed to save settings: {e}"))?;

    log::info!("Commercial trial expiration acknowledged");
    Ok(())
  }

  pub fn has_acknowledged(&self, _app_handle: &AppHandle) -> Result<bool, String> {
    let settings_manager = SettingsManager::instance();
    let settings = settings_manager
      .load_settings()
      .map_err(|e| format!("Failed to load settings: {e}"))?;

    Ok(settings.commercial_trial_acknowledged)
  }
}

lazy_static::lazy_static! {
  static ref COMMERCIAL_LICENSE_MANAGER: CommercialLicenseManager = CommercialLicenseManager;
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_trial_duration() {
    // 2 weeks = 14 * 24 * 60 * 60 = 1,209,600 seconds
    assert_eq!(TRIAL_DURATION_SECONDS, 1_209_600);
  }

  #[test]
  fn test_current_timestamp() {
    let timestamp = CommercialLicenseManager::get_current_timestamp();
    // Timestamp should be after 2020-01-01 (1577836800)
    assert!(timestamp > 1577836800);
  }
}
