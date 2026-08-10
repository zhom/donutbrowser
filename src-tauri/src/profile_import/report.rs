//! What an import actually carried across.
//!
//! Import is best-effort by nature: a locked keychain, a Windows App-Bound
//! cookie store or a schema too old for Chromium to migrate all mean some
//! subset does not survive, and none of them should abort the whole operation.
//! The report is how that stays honest — every skipped store is a counted
//! warning rather than a silent zero.

use serde::{Deserialize, Serialize};

/// Stable warning codes. The frontend maps these to
/// `importProfile.warnings.*`, so they are part of the API contract: rename one
/// and the user sees a missing translation.
pub mod warning {
  /// The source browser's key could not be read, so cookies/passwords were
  /// left encrypted and are unreadable in the new profile.
  pub const SECRETS_NOT_MIGRATED: &str = "secretsNotMigrated";
  /// Windows App-Bound Encryption (Chrome 127+). Unrecoverable by design.
  pub const APP_BOUND_ENCRYPTED: &str = "appBoundEncrypted";
  /// A store's schema predates what Chromium will migrate; it would have been
  /// deleted on first launch, so it was skipped instead.
  pub const STORE_TOO_OLD: &str = "storeTooOld";
  /// A store's schema is newer than this Chromium can read.
  pub const STORE_TOO_NEW: &str = "storeTooNew";
  /// The source browser was running; databases were snapshotted but LevelDB
  /// site data may be incomplete.
  pub const SOURCE_BROWSER_RUNNING: &str = "sourceBrowserRunning";
  /// Tracked preferences lost their MACs and will reset to defaults.
  pub const SECURE_PREFERENCES_RESET: &str = "securePreferencesReset";
  /// At least one extension could not be carried.
  pub const EXTENSIONS_PARTIAL: &str = "extensionsPartial";
  /// A database was unreadable and was skipped rather than copied corrupt.
  pub const STORE_UNREADABLE: &str = "storeUnreadable";
}

/// Per-profile outcome, returned alongside each item in a batch import.
#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ProfileImportReport {
  /// Cookies whose value is readable in the new profile.
  pub cookies_migrated: usize,
  /// Cookies carried over as rows but whose value could not be recovered.
  pub cookies_unrecoverable: usize,
  pub passwords_migrated: usize,
  pub passwords_unrecoverable: usize,
  /// Saved cards / IBANs / autofill secrets re-encrypted.
  pub payment_methods_migrated: usize,
  pub payment_methods_unrecoverable: usize,
  pub extensions_migrated: usize,
  pub history_entries: usize,
  pub bookmarks: usize,
  /// Origins with Local Storage data.
  pub local_storage_origins: usize,
  pub bytes_copied: u64,
  /// Stable codes from [`warning`], deduplicated, in insertion order.
  pub warnings: Vec<String>,
}

impl ProfileImportReport {
  pub fn warn(&mut self, code: &str) {
    if !self.warnings.iter().any(|w| w == code) {
      self.warnings.push(code.to_string());
    }
  }

  /// True when nothing readable came across. Used to decide whether the UI
  /// should present the import as a success or as a warning.
  pub fn is_empty_import(&self) -> bool {
    self.cookies_migrated == 0
      && self.passwords_migrated == 0
      && self.history_entries == 0
      && self.bookmarks == 0
      && self.local_storage_origins == 0
      && self.extensions_migrated == 0
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn warnings_are_deduplicated_in_order() {
    let mut report = ProfileImportReport::default();
    report.warn(warning::STORE_TOO_OLD);
    report.warn(warning::SECRETS_NOT_MIGRATED);
    report.warn(warning::STORE_TOO_OLD);
    assert_eq!(
      report.warnings,
      vec![
        warning::STORE_TOO_OLD.to_string(),
        warning::SECRETS_NOT_MIGRATED.to_string()
      ]
    );
  }

  #[test]
  fn empty_import_detection_ignores_unrecoverable_counts() {
    let mut report = ProfileImportReport {
      cookies_unrecoverable: 500,
      ..Default::default()
    };
    assert!(
      report.is_empty_import(),
      "500 unreadable cookies is still nothing carried"
    );
    report.history_entries = 1;
    assert!(!report.is_empty_import());
  }
}
